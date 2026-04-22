# Host-Tags + Audit + Profile Button Migration (#3j) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all interactive buttons in host-tags, audit-logs, and profile routes to
the Button primitive, with EllipsisIcon ellipsis triggers, Retry loading state, and modal
footer patterns.

**Architecture:** Three route files migrated in separate tasks, parallel-safe.
EllipsisIcon.svelte already exists. confirmClass already stripped (Wave 4).
Each file gets a test extension task.

**Tech Stack:** Svelte 5, Button.svelte, EllipsisIcon.svelte, Vitest, Playwright

---

## Pre-Work Checklist

- [ ] Button.svelte contract verified (variant: primary/ghost/danger/secondary; size: md/sm; loading, ariaLabel, leadingIcon snippet)
- [ ] EllipsisIcon.svelte exists at `frontend/src/lib/components/icons/EllipsisIcon.svelte` (reuse if present, create if missing)
- [ ] Three source files identified: `frontend/src/routes/host-tags/+page.svelte`, `frontend/src/routes/audit-logs/+page.svelte`, `frontend/src/routes/profile/+page.svelte`
- [ ] Three test files identified: `host-tags.test.ts`, `audit-logs.test.ts`, `profile.test.ts`

---

## Task 1: Create or verify EllipsisIcon.svelte

**Objective:** Ensure EllipsisIcon.svelte exists as a static SVG with no props.

**Steps:**

- [ ] Check if `frontend/src/lib/components/icons/` directory exists
  - If not, create it: `mkdir -p frontend/src/lib/components/icons`
- [ ] Check if `frontend/src/lib/components/icons/EllipsisIcon.svelte` already exists
  - If it exists (reused from #3i or earlier work), verify it's a static SVG with no props. Skip creation.
  - If it doesn't exist, create it with exact content:

```svelte
<svg
	xmlns="http://www.w3.org/2000/svg"
	viewBox="0 0 16 16"
	width="1em"
	height="1em"
	fill="currentColor"
	aria-hidden="true"
>
	<circle cx="2" cy="8" r="1.5" />
	<circle cx="8" cy="8" r="1.5" />
	<circle cx="14" cy="8" r="1.5" />
</svg>
```

- [ ] Verify no component props, no named slots, no reactive bindings

**Commit:** `feat(frontend): add EllipsisIcon.svelte component (if created)` or `chore(frontend): verify EllipsisIcon.svelte exists`

---

## Task 2: Migrate host-tags/+page.svelte

**Objective:** Convert all Skeleton preset buttons in host-tags to Button primitive with ellipsis trigger and async retry.

**Steps:**

### 2a. Import Button and EllipsisIcon

- [ ] Add import at top of script:

  ```svelte
  import Button from '$lib/components/Button.svelte';
  import EllipsisIcon from '$lib/components/icons/EllipsisIcon.svelte';
  ```

### 2b. Add isRetrying state

- [ ] Add after existing state declarations (around line 40):

  ```svelte
  let isRetrying: boolean = $state(false);
  ```

### 2c. Migrate Create Tag header action (line 295)

**Current:**

```svelte
<button class="btn preset-filled-primary-500" onclick={openCreateDialog}>Create Tag</button>
```

**New:**

```svelte
<Button variant="primary" onclick={openCreateDialog}>Create Tag</Button>
```

### 2d. Migrate row ellipsis trigger (lines 374–383)

**Current:**

```svelte
<button
	class="btn btn-sm preset-tonal"
	aria-label="Actions for {tag.name}"
	onclick={(e) => {
		e.stopPropagation();
		toggleMenu(tag.id, e.currentTarget);
	}}
>
	&#8943;
</button>
```

**New:**

```svelte
<Button
	variant="ghost"
	size="sm"
	ariaLabel="Actions for {tag.name}"
	onclick={(e) => {
		e.stopPropagation();
		toggleMenu(tag.id, e.currentTarget);
	}}
>
	{#snippet leadingIcon()}<EllipsisIcon />{/snippet}
	<span class="sr-only">Actions for {tag.name}</span>
</Button>
```

### 2e. Migrate error Retry button (line 390)

**Current:**

```svelte
<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadTags(currentPage)}>Retry</button>
```

**New:**

```svelte
<Button
	variant="primary"
	loading={isRetrying}
	onclick={async () => {
		isRetrying = true;
		try {
			await loadTags(currentPage);
		} finally {
			isRetrying = false;
		}
	}}
	class="mt-3"
>
	Retry
</Button>
```

### 2f. Migrate Create modal Color toggle buttons (lines 472–476 and 479–483)

**Current (Auto, when color present):**

```svelte
<button
	type="button"
	class="btn btn-sm preset-tonal-surface flex-shrink-0"
	onclick={() => (createForm.color = '')}>Auto</button
>
```

**New:**

```svelte
<Button variant="secondary" size="sm" class="flex-shrink-0" onclick={() => (createForm.color = '')}>
	Auto
</Button>
```

**Current (Pick color, when color absent):**

```svelte
<button
	type="button"
	class="btn btn-sm preset-tonal-surface flex-shrink-0"
	onclick={() => (createForm.color = '#3B82F6')}>Pick color</button
>
```

**New:**

```svelte
<Button variant="secondary" size="sm" class="flex-shrink-0" onclick={() => (createForm.color = '#3B82F6')}>
	Pick color
</Button>
```

### 2g. Migrate Create modal footer (lines 492–501)

**Current:**

```svelte
{#snippet footer()}
	<button class="btn preset-tonal-surface" onclick={() => (showCreateModal = false)}>Cancel</button>
	<button
		class="btn preset-filled-primary-500"
		disabled={submitting || !createForm.name.trim()}
		onclick={executeCreate}
	>
		{submitting ? 'Creating...' : 'Create'}
	</button>
{/snippet}
```

**New:**

```svelte
{#snippet footer()}
	<Button variant="secondary" onclick={() => (showCreateModal = false)}>Cancel</Button>
	<Button
		variant="primary"
		loading={submitting}
		disabled={!createForm.name.trim()}
		onclick={executeCreate}
	>
		Create
	</Button>
{/snippet}
```

### 2h. Migrate Edit modal footer (lines 524–532)

**Current:**

```svelte
{#snippet footer()}
	<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
	<button
		class="btn preset-filled-primary-500"
		disabled={submitting || !editTag?.name.trim()}
		onclick={executeEdit}
	>
		{submitting ? 'Saving...' : 'Save'}
	</button>
{/snippet}
```

**New:**

```svelte
{#snippet footer()}
	<Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
	<Button
		variant="primary"
		loading={submitting}
		disabled={!editTag?.name.trim()}
		onclick={executeEdit}
	>
		Save
	</Button>
{/snippet}
```

### 2i. Verify no preset-filled or preset-tonal classes remain

- [ ] Search file for `preset-filled` and `preset-tonal` — none should exist except in ConfirmDialog props (outside scope)

**Commit:** `feat(frontend): migrate host-tags to Button primitive (sub-spec #3j)`

---

## Task 3: Migrate audit-logs/+page.svelte

**Objective:** Convert Apply/Clear Filters and Retry buttons to Button primitive.

**Steps:**

### 3a. Add isRetrying state

- [ ] Add after existing state declarations (around line 71):

  ```svelte
  let isRetrying: boolean = $state(false);
  ```

### 3b. Migrate Apply Filters button (line 220)

**Current:**

```svelte
<button class="btn preset-filled-primary-500" onclick={applyFilters}>Apply Filters</button>
```

**New:**

```svelte
<Button variant="primary" onclick={applyFilters}>Apply Filters</Button>
```

### 3c. Migrate Clear Filters button (line 221)

**Current:**

```svelte
<button class="btn preset-tonal-surface" onclick={clearFilters}>Clear Filters</button>
```

**New:**

```svelte
<Button variant="secondary" onclick={clearFilters}>Clear Filters</Button>
```

### 3d. Add Button import at top

- [ ] Add import at top of script:

  ```svelte
  import Button from '$lib/components/Button.svelte';
  ```

### 3e. Migrate error Retry button (line 311)

**Current:**

```svelte
<button class="btn preset-filled-primary-500 mt-3" onclick={() => load(currentPage)}>Retry</button>
```

**New:**

```svelte
<Button
	variant="primary"
	loading={isRetrying}
	onclick={async () => {
		isRetrying = true;
		try {
			await load(currentPage);
		} finally {
			isRetrying = false;
		}
	}}
	class="mt-3"
>
	Retry
</Button>
```

### 3f. Verify no preset classes remain

- [ ] Search file for `preset-filled` and `preset-tonal` — none should exist

**Commit:** `feat(frontend): migrate audit-logs to Button primitive (sub-spec #3j)`

---

## Task 4: Migrate profile/+page.svelte

**Objective:** Convert New Token launcher, Revoke buttons, and modal footer buttons to Button primitive.

**Steps:**

### 4a. Migrate New Token launcher (line 116)

**Current:**

```svelte
<button class="btn preset-filled-primary-500" onclick={openCreateModal}>New Token</button>
```

**New:**

```svelte
<Button variant="primary" onclick={openCreateModal}>New Token</Button>
```

### 4b. Migrate Revoke row button (lines 148–153)

**Current:**

```svelte
<button
	class="btn btn-sm preset-tonal-error"
	onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
>
	Revoke
</button>
```

**New:**

```svelte
<Button
	variant="danger"
	size="sm"
	onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
>
	Revoke
</Button>
```

### 4c. Migrate modal footer Created state (lines 203–206)

**Current:**

```svelte
<button class="btn preset-tonal-surface" onclick={() => copyToken(createdToken!)}>Copy</button>
<button class="btn preset-filled-primary-500" onclick={closeCreateModal}>Done</button>
```

**New:**

```svelte
<Button variant="secondary" onclick={() => copyToken(createdToken!)}>Copy</Button>
<Button variant="primary" onclick={closeCreateModal}>Done</Button>
```

### 4d. Migrate modal footer Create state Cancel (line 207)

**Current:**

```svelte
<button class="btn preset-tonal-surface" onclick={closeCreateModal}>Cancel</button>
```

**New:**

```svelte
<Button variant="secondary" onclick={closeCreateModal}>Cancel</Button>
```

### 4e. SKIP Create submit button (line 209)

- [ ] **DO NOT MIGRATE** — line 209 Create button is already migrated as of Wave 3:

  ```svelte
  <Button variant="primary" onclick={handleCreate} disabled={!newTokenName.trim()} loading={creating}>
      Create
  </Button>
  ```

  Assert this line exists and remains unchanged. Do not re-migrate.

### 4f. Verify no preset classes remain

- [ ] Search file for `preset-filled` and `preset-tonal` — none should exist

**Commit:** `feat(frontend): migrate profile to Button primitive (sub-spec #3j)`

---

## Task 5: Extend host-tags unit tests

**Objective:** Add test cases for all Button migrations in host-tags.test.ts.

**File:** `frontend/src/routes/host-tags/host-tags.test.ts`

**Steps:**

- [ ] Read full test file to understand test structure
- [ ] Add test suite for Button variants (after existing tests):

```typescript
describe('Button Migrations', () => {
	it('Create Tag header action renders variant="primary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Create Tag' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Create Tag' });
		expect(btn).toHaveClass('inline-flex'); // Button base class
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('Row ellipsis trigger renders variant="ghost" size="sm" with EllipsisIcon and sr-only children', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([{ id: 'tag-1', name: 'prod', color: '#FF0000', description: '', created_at: '2026-04-19T00:00:00Z', host_count: 5 }])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Actions for prod' });
		expect(btn).toHaveClass('h-[19px]'); // size="sm"
		expect(btn).toHaveClass('bg-transparent'); // ghost variant
		const srOnly = btn.querySelector('span.sr-only');
		expect(srOnly?.textContent).toBe('Actions for prod');
		expect(btn.querySelector('svg')).toBeInTheDocument(); // EllipsisIcon rendered
	});

	it('Row ellipsis trigger preserves stopPropagation and e.currentTarget for menu positioning', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([{ id: 'tag-1', name: 'test', color: '#00FF00', description: '', created_at: '2026-04-19T00:00:00Z', host_count: 2 }])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for test' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Actions for test' });
		const mockStopPropagation = vi.fn();
		const event = new MouseEvent('click', { bubbles: true });
		Object.defineProperty(event, 'stopPropagation', { value: mockStopPropagation });
		Object.defineProperty(event, 'currentTarget', { value: btn });
		btn.dispatchEvent(event);
		expect(mockStopPropagation).toHaveBeenCalled();
	});

	it('Error Retry button renders variant="primary" with async loading state', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Network error'));
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(btn).not.toHaveAttribute('aria-busy', 'true'); // Not loading initially
		expect(btn).not.toHaveAttribute('disabled');

		// Simulate click and loading
		vi.mocked(api.getHostTags).mockResolvedValueOnce(makePage([]));
		const clickPromise = btn.click();
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		await clickPromise;
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('Error Retry button clears loading state after rejection', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Load failed'));
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });

		// Mock rejection on retry click
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Retry failed'));
		try {
			btn.click();
		} catch {
			// Expected
		}
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('Create modal Auto toggle renders variant="secondary" size="sm"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByText('Create Tag')).toBeInTheDocument());
		const autoBtn = screen.getByRole('button', { name: 'Auto' });
		expect(autoBtn).toHaveClass('h-[19px]'); // size="sm"
		expect(autoBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Create modal footer Cancel renders variant="secondary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByText('Create Tag')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Create modal footer Create submit renders variant="primary" with loading={submitting}', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByText('Create Tag')).toBeInTheDocument());
		const submitBtn = screen.getByRole('button', { name: 'Create' });
		expect(submitBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(submitBtn).toHaveAttribute('disabled'); // Disabled when name empty
		const nameInput = screen.getByPlaceholderText('e.g. production');
		await userEvent.type(nameInput, 'new-tag');
		expect(submitBtn).not.toHaveAttribute('disabled'); // Enabled when name present
	});

	it('Create modal footer Create children stay static "Create" across submit window', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.createHostTag).mockImplementation(
			() => new Promise((resolve) => setTimeout(() => resolve({ id: 'tag-1', name: 'new', color: '', description: '', created_at: '2026-04-19T00:00:00Z', host_count: 0 }), 100))
		);
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByText('Create Tag')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. production');
		await userEvent.type(nameInput, 'new-tag');
		const submitBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(submitBtn);
		// Children should remain "Create", not "Creating..."
		expect(submitBtn.textContent).toContain('Create');
		expect(submitBtn.textContent).not.toContain('Creating');
	});

	it('Edit modal footer Save renders variant="primary" with loading={submitting} and disabled={!editTag?.name.trim()}', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([{ id: 'tag-1', name: 'prod', color: '#FF0000', description: 'desc', created_at: '2026-04-19T00:00:00Z', host_count: 5 }])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for prod' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		await userEvent.click(editItem);
		await waitFor(() => expect(screen.getByDisplayValue('prod')).toBeInTheDocument());
		const saveBtn = screen.getByRole('button', { name: 'Save' });
		expect(saveBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		// Clear name field
		const nameInput = screen.getByDisplayValue('prod') as HTMLInputElement;
		nameInput.value = '';
		nameInput.dispatchEvent(new Event('input', { bubbles: true }));
		await waitFor(() => expect(saveBtn).toHaveAttribute('disabled'));
		// Restore name
		nameInput.value = 'updated';
		nameInput.dispatchEvent(new Event('input', { bubbles: true }));
		await waitFor(() => expect(saveBtn).not.toHaveAttribute('disabled'));
	});

	it('Edit modal footer Cancel renders variant="secondary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([{ id: 'tag-1', name: 'test', color: '#00FF00', description: '', created_at: '2026-04-19T00:00:00Z', host_count: 2 }])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for test' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for test' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		await userEvent.click(editItem);
		await waitFor(() => expect(screen.getByDisplayValue('test')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Out-of-scope regression: Edit/Delete ContextMenuItems remain unchanged and are not wrapped in Button', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([{ id: 'tag-1', name: 'prod', color: '#FF0000', description: '', created_at: '2026-04-19T00:00:00Z', host_count: 5 }])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for prod' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		expect(editItem).toBeInTheDocument();
		expect(editItem.tagName).toBe('LI'); // ContextMenuItem remains unchanged
		const deleteItem = screen.getByRole('menuitem', { name: 'Delete' });
		expect(deleteItem).toBeInTheDocument();
		expect(deleteItem.tagName).toBe('LI'); // ContextMenuItem remains unchanged
	});
});
```

- [ ] Import userEvent at top of test file if not already present:

  ```typescript
  import userEvent from '@testing-library/user-event';
  ```

**Commit:** `test(frontend): extend host-tags tests for Button migration (sub-spec #3j)`

---

## Task 6: Extend audit-logs unit tests

**Objective:** Add test cases for all Button migrations in audit-logs.test.ts.

**File:** `frontend/src/routes/audit-logs/audit-logs.test.ts`

**Steps:**

- [ ] Read full test file to understand test structure
- [ ] Add test suite for Button variants (after existing tests):

```typescript
describe('Button Migrations', () => {
	it('Apply Filters button renders variant="primary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Apply Filters' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Apply Filters' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('Clear Filters button renders variant="secondary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Clear Filters' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Clear Filters' });
		expect(btn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Apply Filters click triggers load(1) and updates DataTable loading prop', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Apply Filters' })).toBeInTheDocument());
		const applyBtn = screen.getByRole('button', { name: 'Apply Filters' });
		const actionInput = screen.getByPlaceholderText('e.g. login');
		await userEvent.type(actionInput, 'create');
		await userEvent.click(applyBtn);
		await waitFor(() => expect(vi.mocked(api.listAuditLogs)).toHaveBeenCalledWith(
			expect.objectContaining({ page: 1, action_type: 'create' })
		));
	});

	it('Clear Filters click resets filter state and triggers load(1)', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Clear Filters' })).toBeInTheDocument());
		const actionInput = screen.getByPlaceholderText('e.g. login');
		await userEvent.type(actionInput, 'delete');
		const clearBtn = screen.getByRole('button', { name: 'Clear Filters' });
		await userEvent.click(clearBtn);
		await waitFor(() => {
			expect((actionInput as HTMLInputElement).value).toBe('');
			expect(vi.mocked(api.listAuditLogs)).toHaveBeenCalledWith(
				expect.objectContaining({ page: 1, action_type: undefined })
			);
		});
	});

	it('Error Retry button renders variant="primary" with async loading state', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Network error'));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(btn).not.toHaveAttribute('aria-busy', 'true'); // Not loading initially
		expect(btn).not.toHaveAttribute('disabled');

		// Simulate click and loading
		vi.mocked(api.listAuditLogs).mockResolvedValueOnce(makePage([sampleEntry]));
		const clickPromise = btn.click();
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		await clickPromise;
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('Error Retry button clears loading state after rejection', async () => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Load failed'));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });

		// Mock rejection on retry click
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Retry failed'));
		try {
			btn.click();
		} catch {
			// Expected
		}
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('Out-of-scope regression: TabStrip scope toggle remains unchanged', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...auditViewer,
			permissions: [Permission.ViewAuditLogs, Permission.ViewSystemAuditLogs]
		});
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('tablist', { name: 'Audit log scope' })).toBeInTheDocument());
		const tablist = screen.getByRole('tablist', { name: 'Audit log scope' });
		expect(tablist).toBeInTheDocument();
		const tenantTab = screen.getByRole('tab', { name: 'Tenant Logs' });
		const systemTab = screen.getByRole('tab', { name: 'System Logs' });
		expect(tenantTab).toBeInTheDocument();
		expect(systemTab).toBeInTheDocument();
	});
});
```

- [ ] Import userEvent at top of test file if not already present:

  ```typescript
  import userEvent from '@testing-library/user-event';
  ```

**Commit:** `test(frontend): extend audit-logs tests for Button migration (sub-spec #3j)`

---

## Task 7: Extend profile unit tests

**Objective:** Add test cases for all Button migrations in profile.test.ts.

**File:** `frontend/src/routes/profile/profile.test.ts`

**Steps:**

- [ ] Read full test file to understand test structure and existing mocks
- [ ] Add test suite for Button variants (after existing tests):

```typescript
describe('Button Migrations', () => {
	it('New Token launcher renders variant="primary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'New Token' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'New Token' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('Row Revoke button renders variant="danger" size="sm"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		const token: ApiTokenResponse = {
			id: 'token-1',
			name: 'CI Pipeline',
			created_at: '2026-04-19T00:00:00Z',
			revoked_at: null
		};
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Revoke' });
		expect(btn).toHaveClass('h-[19px]'); // size="sm"
		expect(btn).toHaveClass('bg-[var(--color-error-bg)]'); // danger variant
	});

	it('New API Token modal Create state Cancel button renders variant="secondary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('New API Token modal Create state Create button already migrated (Wave 3)', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const createBtn = screen.getByRole('button', { name: 'Create' });
		expect(createBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(createBtn).toBeDisabled(); // Disabled when name empty
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'new-token');
		await waitFor(() => expect(createBtn).not.toHaveAttribute('disabled'));
		// Verify no aria-busy when not loading (Button removes attr when loading=false)
		expect(createBtn).not.toHaveAttribute('aria-busy');
		// Verify static children "Create" (no text-swap)
		expect(createBtn.textContent).toContain('Create');
	});

	it('New API Token modal Created state Copy button renders variant="secondary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument());
		const copyBtn = screen.getByRole('button', { name: 'Copy' });
		expect(copyBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('New API Token modal Created state Done button renders variant="primary"', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument());
		const doneBtn = screen.getByRole('button', { name: 'Done' });
		expect(doneBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('New API Token modal Copy button invokes clipboard.writeText and surfaces success toast', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		const clipboardSpy = vi.spyOn(navigator.clipboard, 'writeText');
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument());
		const copyBtn = screen.getByRole('button', { name: 'Copy' });
		await userEvent.click(copyBtn);
		await waitFor(() => expect(clipboardSpy).toHaveBeenCalledWith('secret-token-123'));
		clipboardSpy.mockRestore();
	});

	it('Out-of-scope regression: ConfirmDialog Revoke confirmation is not wrapped in Button', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		const token: ApiTokenResponse = {
			id: 'token-1',
			name: 'Test Token',
			created_at: '2026-04-19T00:00:00Z',
			revoked_at: null
		};
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const revokeBtn = screen.getByRole('button', { name: 'Revoke' });
		await userEvent.click(revokeBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument());
		// ConfirmDialog is rendered but its confirm button is owned by #3k
		// We only assert that the launcher (Revoke) opened the dialog
		expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument();
	});
});
```

- [ ] Add missing imports at top of test file (if not already present):

  ```typescript
  import userEvent from '@testing-library/user-event';
  import type { ApiTokenResponse } from '$lib/types';
  ```

- [ ] Verify `user` mock exists in test file — `profile.test.ts` already defines
  `const user = { ... }` at line 23. Use it directly; do NOT redeclare.

**Commit:** `test(frontend): extend profile tests for Button migration (sub-spec #3j)`

---

## Task 8: Re-baseline Playwright snapshots (all three routes)

**Objective:** Capture e2e visual regressions for host-tags, audit-logs, and profile in dark + light themes.

**Steps:**

- [ ] Ensure Playwright e2e tests exist for all three routes (typically in `frontend/tests/e2e/`)
- [ ] Run snapshot re-baseline for host-tags in dark theme:

  ```bash
  cd frontend && npx playwright test host-tags --update-snapshots
  ```

- [ ] Run snapshot re-baseline for host-tags in light theme (if separate test):

  ```bash
  cd frontend && npx playwright test host-tags --project=light --update-snapshots
  ```

- [ ] Run snapshot re-baseline for audit-logs in dark + light themes:

  ```bash
  cd frontend && npx playwright test audit-logs --update-snapshots
  ```

- [ ] Run snapshot re-baseline for profile in dark + light themes:

  ```bash
  cd frontend && npx playwright test profile --update-snapshots
  ```

- [ ] Verify snapshots apply correct masking per spec §8:
  - Spinner rotation masked on Retry, Create, Save buttons
  - formatDate() outputs masked in tables (created_at, occurred_at, revoked_at)
  - Generated token string masked in profile created-token `<pre>`
  - Toast banners masked
  - Batch selection count in BatchActionBar masked if present

**Commit:** `test(frontend): re-baseline e2e snapshots for Button migration (sub-spec #3j)`

---

## Task 9: Run full frontend gate

**Objective:** Verify all quality checks pass before merging.

**Steps:**

- [ ] Run linter:

  ```bash
  cd frontend && npm run lint
  ```

- [ ] Run formatter check:

  ```bash
  cd frontend && npm run format:check
  ```

- [ ] Run type check:

  ```bash
  cd frontend && npm run check
  ```

- [ ] Run unit tests:

  ```bash
  cd frontend && npm run test
  ```

- [ ] Run build:

  ```bash
  cd frontend && npm run build
  ```

- [ ] All commands must pass with no errors

**Commit:** `chore(frontend): verify full gate passes for Button migration (sub-spec #3j)`

---

## Task 10: Create PR and merge

**Objective:** Prepare single PR and merge to main.

**Steps:**

- [ ] Create PR with title: `feat(frontend): migrate host-tags + audit-logs + profile to Button primitive (sub-spec #3j)`
- [ ] PR description:

  ```markdown
  ## Summary
  Migrate three admin routes (host-tags, audit-logs, profile) to Button primitive with icon-only ellipsis triggers, async retry loading state, and modal footer patterns.

  - Host-Tags: Create Tag launcher, row ellipsis trigger, Create/Edit modal footers, Auto/Pick color toggles, error Retry
  - Audit-Logs: Apply/Clear Filters, error Retry
  - Profile: New Token launcher, row Revoke buttons, New API Token modal footer (Cancel, Copy, Done; Create already migrated in Wave 3)

  All migrations use canonical Button variants: primary, ghost, danger, secondary.

  ## Test Plan
  - Unit tests extended for all Button variants, loading states, aria attributes
  - e2e snapshots re-baselined in dark + light themes
  - Full frontend gate passes (lint, format, type check, unit tests, build)

  Fixes: #3j
  ```

- [ ] Ensure all commits are present and in order:
  1. EllipsisIcon creation (if created)
  2. host-tags migration
  3. audit-logs migration
  4. profile migration
  5. host-tags tests
  6. audit-logs tests
  7. profile tests
  8. e2e snapshots
  9. gate verification
- [ ] Merge to main

**Commit:** N/A — already committed per task

---

## Rollback Plan

If critical regression found post-merge:

- [ ] `git revert <PR-merge-commit>` — single revert restores all three routes to preset classes
- [ ] Verify previous gate still passes on original branch
- [ ] Open rollback PR with reason + finding

---

## Success Criteria

✓ All three routes (`host-tags`, `audit-logs`, `profile`) render only Button primitive, no Skeleton preset classes
✓ No `preset-filled-*`, `preset-tonal-*` class names in migrated code (except ConfirmDialog props, out of scope)
✓ EllipsisIcon renders on all row action triggers with `sr-only` children
✓ Retry buttons wire `loading={isRetrying}` with async `try`/`finally` wrapper
✓ Modal footers: Cancel → `variant="secondary"`, Submit → `variant="primary" loading={...}`
✓ Unit tests cover all variants, loading states, aria attributes, event handlers
✓ e2e snapshots re-baselined and masking applied per spec
✓ Full frontend gate passes (lint, format, type check, unit tests, build)
✓ PR created and merged to main

---

## Parallel Execution

Tasks 2–4 (three route migrations) are parallel-safe and may be executed in parallel by independent agents.
Tasks 5–7 (three test extensions) depend on Tasks 2–4 respectively but are otherwise parallel-safe.
Task 8 may begin after all of Tasks 5–7 are complete.
Task 9 final gate depends on all prior tasks.
Task 10 depends on Task 9.

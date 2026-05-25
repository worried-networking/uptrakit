<!-- markdownlint-disable MD013 -->

# Forms, Modals & Surfaces Rework — Plan B: Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement all frontend changes from the forms/modals/surfaces rework spec: submit-label defaults, title casing, button right-alignment, modal-trigger buttons in card headers, load/refresh button removal, SurfaceRenderer section header actions, SchemaForm draft mode, and design-language doc updates.

**Architecture:** Pure frontend changes in `frontend/src/`. Plan A (Rust + contract) must land first — Task 5 (SurfaceRenderer) consumes the `header_action_ids` field and `submit_label` field added to the TypeScript contract in Plan A. All other tasks are independent of Plan A. Svelte 5 runes (`$state`, `$derived`, `{#snippet}`) throughout. Draft mode in SchemaForm uses `createFormDraft` from `$lib/forms/draft.svelte` — do NOT re-implement dirty tracking inline (spec requirement). `SurfaceForm.svelte`'s `submitInteraction` must return `Promise<unknown>` so SchemaForm can call `form.commit()` on success.

**Tech Stack:** SvelteKit 2.60+, Svelte 5 runes, TypeScript, Tailwind CSS, `@testing-library/svelte`, Vitest. Verify with `cd frontend && npm run check && npm run lint && npm run test`.

---

## File Map

| File                                                           | Task(s) | Action                                                                                                              |
| -------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/components/surfaces/SurfaceForm.svelte`      | T1, T6  | Modify: `submitLabel` default + `interaction.submit_label` priority; `submitInteraction` returns `Promise<unknown>` |
| `frontend/src/lib/components/surfaces/SchemaForm.svelte`       | T1, T6  | Modify: `submitLabel` default, draft mode via `createFormDraft`, button row                                         |
| `frontend/src/lib/components/surfaces/SurfaceForm.test.ts`     | T1      | Modify: add submit-label default test                                                                               |
| `frontend/src/lib/components/surfaces/SchemaForm.test.ts`      | T1, T6  | Modify: add submit-label + draft-mode tests                                                                         |
| `frontend/src/routes/settings/McpAccessTab.svelte`             | T2, T3  | Modify: title casing + button alignment                                                                             |
| `frontend/src/routes/settings/AgentCertificateSettings.svelte` | T3      | Modify: `flex gap-2` → `flex gap-2 justify-end`                                                                     |
| `frontend/src/routes/settings/GlobalSettingsTab.svelte`        | T3      | Modify: standalone buttons wrapped with `justify-end`                                                               |
| `frontend/src/routes/settings/DangerZone.svelte`               | T3      | Modify: Reset Data button right-aligned                                                                             |
| `frontend/src/routes/settings/OidcProvidersSettings.svelte`    | T4, T8  | Modify: "Add Provider" moves to `{#snippet actions()}`; modal footer "Create"/"Update" → "Save"                     |
| `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`  | T4, T8  | Modify: "Create Token" to header, remove Load/Refresh; modal footer "Create" → "Save"                               |
| `frontend/src/routes/settings/SystemServicesSettings.svelte`   | T4, T8  | Modify: "Create Token" to header, remove Load/Refresh; modal footer "Create" → "Save"                               |
| `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts` | T4      | Modify: remove "Load Tokens" click, wait for auto-load                                                              |
| `frontend/src/routes/settings/SystemServicesSettings.test.ts`  | T4      | Modify: remove "Load Tokens" click, wait for auto-load                                                              |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`  | T5      | Modify: section branch → conditional SectionCard when `header_action_ids`                                           |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts` | T5      | Modify: add header_action_ids test                                                                                  |
| `docs/development/ui/surfaces.md`                              | T7      | Modify: Section primitive, header action contract, Form draft mode                                                  |
| `docs/development/ui/primitives.md`                            | T7      | Modify: SectionCard button placement rules, form action alignment                                                   |

---

### Task 1: Submit Label Defaults ('Submit' → 'Save')

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.svelte:21,42`
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.svelte:12`
- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.test.ts`

- [ ] **Step 1: Write failing tests**

Add to `frontend/src/lib/components/surfaces/SchemaForm.test.ts`:

```typescript
it("renders Save as default submit label", () => {
  render(SchemaForm, {
    fields: [
      { key: "name", label: "Name", field_type: "text", required: false },
    ],
    onsubmit: vi.fn().mockResolvedValue(undefined),
  });
  expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
});
```

> **Prerequisite:** The two SurfaceForm tests below reference `InteractionDescriptor.submit_label`, which is added to `contract.ts` in **Plan A**. Write these tests now, but they will only pass `npm run check` after Plan A is merged. The SchemaForm default-label test (above) has no prerequisite and can be committed independently if needed.

Add to `frontend/src/lib/components/surfaces/SurfaceForm.test.ts` (in the existing `describe('SurfaceForm')` block):

```typescript
it("renders Save as default submit label when interaction has no submit_label", async () => {
  vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
  const interaction: InteractionDescriptor = {
    interaction_id: "save-form",
    kind: "form_submit",
    label: "Save Settings",
    transport: { mode: "controller_local" },
    form_ui: {
      fields: [
        { key: "value", label: "Value", field_type: "text", required: false },
      ],
    },
  };
  render(SurfaceForm, {
    surfaceId: "test",
    interaction,
    interactions: [interaction],
  });
  expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
});

it("uses interaction.submit_label when present", async () => {
  vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
  const interaction: InteractionDescriptor = {
    interaction_id: "rotate",
    kind: "form_submit",
    label: "Rotate CA",
    submit_label: "Rotate CA",
    transport: { mode: "controller_local" },
    form_ui: {
      fields: [
        {
          key: "confirm",
          label: "Confirm",
          field_type: "text",
          required: false,
        },
      ],
    },
  };
  render(SurfaceForm, {
    surfaceId: "test",
    interaction,
    interactions: [interaction],
  });
  expect(screen.getByRole("button", { name: "Rotate CA" })).toBeInTheDocument();
});
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SchemaForm.test.ts src/lib/components/surfaces/SurfaceForm.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓|Save|Submit"
```

Expected: tests referencing 'Save' fail because button currently shows 'Submit'.

- [ ] **Step 3: Update SchemaForm.svelte submit label default**

In `frontend/src/lib/components/surfaces/SchemaForm.svelte` line 12, change:

```typescript
submitLabel = 'Submit',
```

to:

```typescript
submitLabel = 'Save',
```

- [ ] **Step 4: Update SurfaceForm.svelte submit label default and interaction.submit_label priority**

In `frontend/src/lib/components/surfaces/SurfaceForm.svelte`:

Line 21 — change prop default:

```typescript
submitLabel = 'Submit',
```

to:

```typescript
submitLabel = 'Save',
```

Line 42 — change `effectiveSubmitLabel` to read `interaction.submit_label` first:

```typescript
const effectiveSubmitLabel = $derived(submitLabel?.trim() || "Submit");
```

to:

```typescript
const effectiveSubmitLabel = $derived(
  interaction.submit_label?.trim() || submitLabel?.trim() || "Save",
);
```

- [ ] **Step 5: Run tests — verify they pass**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SchemaForm.test.ts src/lib/components/surfaces/SurfaceForm.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: all tests pass.

- [ ] **Step 6: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceForm.svelte \
        frontend/src/lib/components/surfaces/SchemaForm.svelte \
        frontend/src/lib/components/surfaces/SurfaceForm.test.ts \
        frontend/src/lib/components/surfaces/SchemaForm.test.ts
git commit -m "$(cat <<'EOF'
fix(frontend): default submit label to 'Save'; read interaction.submit_label

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: SectionCard Title Casing

**Files:**

- Modify: `frontend/src/routes/settings/McpAccessTab.svelte:188,282`

- [ ] **Step 0: Audit all SectionCard titles (spec R4 requirement)**

Run the audit grep across the full frontend source:

```bash
grep -rn 'title="' frontend/src/routes/ frontend/src/lib/ | grep -v '\.test\.' | grep -E 'title="[A-Za-z]+ [a-z]'
```

Review every match. Apply title case to any string that has a lowercase second-or-later major word. Known violations are fixed in the steps below; any additional matches found by this grep must also be fixed before committing.

- [ ] **Step 1: Fix "Registered clients" → "Registered Clients"**

In `frontend/src/routes/settings/McpAccessTab.svelte` line 188:

```svelte
<SectionCard title="Registered clients">
```

→

```svelte
<SectionCard title="Registered Clients">
```

- [ ] **Step 2: Fix "OAuth settings" → "OAuth Settings"**

In `frontend/src/routes/settings/McpAccessTab.svelte` line 282:

```svelte
<SectionCard title="OAuth settings">
```

→

```svelte
<SectionCard title="OAuth Settings">
```

- [ ] **Step 3: TypeScript check + lint**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/settings/McpAccessTab.svelte
git commit -m "$(cat <<'EOF'
fix(frontend): title-case SectionCard titles in McpAccessTab

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Bottom-Right Form Action Button Alignment

**Files:**

- Modify: `frontend/src/routes/settings/AgentCertificateSettings.svelte:111`
- Modify: `frontend/src/routes/settings/McpAccessTab.svelte:343`
- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte:365,401,460,523,539,553`
- Modify: `frontend/src/routes/settings/DangerZone.svelte:68-71`

- [ ] **Step 1: AgentCertificateSettings — add justify-end**

In `frontend/src/routes/settings/AgentCertificateSettings.svelte` line 111:

```svelte
<div class="flex gap-2">
```

→

```svelte
<div class="flex gap-2 justify-end">
```

- [ ] **Step 2: McpAccessTab — OAuth settings save row**

In `frontend/src/routes/settings/McpAccessTab.svelte` line 343 (the `<div class="flex gap-2">` wrapping the Save/Discard buttons in the OAuth settings section):

```svelte
<div class="flex gap-2">
```

→

```svelte
<div class="flex gap-2 justify-end">
```

- [ ] **Step 3: GlobalSettingsTab — GitHub Provider save row**

In `frontend/src/routes/settings/GlobalSettingsTab.svelte` line 365:

```svelte
<div class="flex flex-wrap gap-2">
```

→

```svelte
<div class="flex flex-wrap gap-2 justify-end">
```

- [ ] **Step 4: GlobalSettingsTab — NATS save/clear row**

In `frontend/src/routes/settings/GlobalSettingsTab.svelte` line 401:

```svelte
<div class="flex gap-2">
```

→

```svelte
<div class="flex gap-2 justify-end">
```

- [ ] **Step 5: GlobalSettingsTab — Zeroconf, Network, TLS Certificate, CA standalone buttons**

Four standalone `<Button>` elements in card bodies need wrapping divs. Apply each change:

Line 460 — Zeroconf Save button:

```svelte
<Button variant="primary" loading={zeroconfSaving} onclick={saveZeroconfSettings}>Save</Button>
```

→

```svelte
<div class="flex justify-end">
  <Button variant="primary" loading={zeroconfSaving} onclick={saveZeroconfSettings}>Save</Button>
</div>
```

Line 523 — Network Settings Save button:

```svelte
<Button variant="primary" onclick={saveNetworkSettings}>Save</Button>
```

→

```svelte
<div class="flex justify-end">
  <Button variant="primary" onclick={saveNetworkSettings}>Save</Button>
</div>
```

Line 539 — Controller TLS Certificate renew button:

```svelte
<Button variant="primary" loading={renewingCert} onclick={handleRenewServerCert}>Renew Server Certificate</Button>
```

→

```svelte
<div class="flex justify-end">
  <Button variant="primary" loading={renewingCert} onclick={handleRenewServerCert}>Renew Server Certificate</Button>
</div>
```

Line 553 — CA Certificate Rotate CA button (confirm-dialog trigger, stays in body but right-aligned):

```svelte
<Button variant="danger" loading={rotatingCa} onclick={() => (showRotateCaConfirm = true)}>Rotate CA</Button>
```

→

```svelte
<div class="flex justify-end">
  <Button variant="danger" loading={rotatingCa} onclick={() => (showRotateCaConfirm = true)}>Rotate CA</Button>
</div>
```

- [ ] **Step 6: DangerZone — Reset Data button (confirm-dialog trigger, stays in body but right-aligned)**

In `frontend/src/routes/settings/DangerZone.svelte` lines 68-71, replace:

```svelte
<Button variant="danger" disabled={!getIsOnline()} onclick={openDialog}>Reset Data</Button>
{#if !getIsOnline()}
  <span class="text-[var(--color-warning)] text-sm ml-2">Offline</span>
{/if}
```

with:

```svelte
<div class="flex items-center gap-2 justify-end">
  {#if !getIsOnline()}
    <span class="text-[var(--color-warning)] text-sm">Offline</span>
  {/if}
  <Button variant="danger" disabled={!getIsOnline()} onclick={openDialog}>Reset Data</Button>
</div>
```

- [ ] **Step 7: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/settings/AgentCertificateSettings.svelte \
        frontend/src/routes/settings/McpAccessTab.svelte \
        frontend/src/routes/settings/GlobalSettingsTab.svelte \
        frontend/src/routes/settings/DangerZone.svelte
git commit -m "$(cat <<'EOF'
fix(frontend): right-align form action buttons across settings components

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Modal-Trigger Buttons to SectionCard Header + Remove Load/Refresh

**Files:**

- Modify: `frontend/src/routes/settings/OidcProvidersSettings.svelte:214-217`
- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte:197-214`
- Modify: `frontend/src/routes/settings/SystemServicesSettings.svelte:181-198`
- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts:53`
- Modify: `frontend/src/routes/settings/SystemServicesSettings.test.ts:53`

- [ ] **Step 1: Update EnrollmentTokenSettings.test.ts — remove "Load Tokens" click**

`EnrollmentTokenSettings.svelte` auto-loads on `onMount` via `void loadTokens()`. After R7 removes the "Load Tokens" button, the test at line 53 that clicks it must instead wait for the auto-load.

In `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts`, find the test "Revoke button has danger class" and replace:

```typescript
render(EnrollmentTokenSettings, props);
// Load tokens to show the table
await fireEvent.click(screen.getByRole("button", { name: "Load Tokens" }));
const revokeBtn = await screen.findByRole("button", { name: "Revoke" });
```

with:

```typescript
render(EnrollmentTokenSettings, props);
// Tokens auto-load on mount — wait for Revoke to appear
const revokeBtn = await screen.findByRole("button", { name: "Revoke" });
```

- [ ] **Step 2: Update SystemServicesSettings.test.ts — remove "Load Tokens" click**

In `frontend/src/routes/settings/SystemServicesSettings.test.ts`, find the test "Revoke button has danger class" and replace:

```typescript
render(SystemServicesSettings, props);
await fireEvent.click(screen.getByRole("button", { name: "Load Tokens" }));
const revokeBtn = await screen.findByRole("button", { name: "Revoke" });
```

with:

```typescript
render(SystemServicesSettings, props);
const revokeBtn = await screen.findByRole("button", { name: "Revoke" });
```

- [ ] **Step 3: Run updated tests — verify they still describe correct behavior**

```bash
cd frontend && npx vitest run --reporter=verbose src/routes/settings/EnrollmentTokenSettings.test.ts src/routes/settings/SystemServicesSettings.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: currently passing (Load Tokens still exists). Tests now pass without clicking Load Tokens — this is correct since they auto-load.

- [ ] **Step 4: OidcProvidersSettings — move "Add Provider" to SectionCard header**

In `frontend/src/routes/settings/OidcProvidersSettings.svelte`, replace lines 214-218:

```svelte
<SectionCard title="OIDC Providers">
	<div class="mb-4 flex items-center justify-between">
		<Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
	</div>
```

with:

```svelte
<SectionCard title="OIDC Providers">
	{#snippet actions()}
		<Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
	{/snippet}
```

- [ ] **Step 5: EnrollmentTokenSettings — move "Create Token" to header, remove Load/Refresh**

In `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`, replace lines 197-214 (the `<SectionCard>` opening tag plus the button `<div>`):

```svelte
<SectionCard title="Enrollment Tokens">
	<div class="mb-4 flex items-center justify-between">
		<div class="flex gap-2">
			{#if tokens === null}
				<Button variant="primary" {loading} disabled={loading} onclick={() => void loadTokens(1)}>Load Tokens</Button>
			{:else}
				<Button variant="secondary" {loading} disabled={loading} onclick={() => void loadTokens(currentPage)}>
					Refresh
				</Button>
			{/if}
			<Button
				variant="primary"
				onclick={() => {
					showCreateDialog = true;
				}}>Create Token</Button
			>
		</div>
	</div>
```

with:

```svelte
<SectionCard title="Enrollment Tokens">
	{#snippet actions()}
		<Button
			variant="primary"
			onclick={() => {
				showCreateDialog = true;
			}}>Create Token</Button
		>
	{/snippet}
```

- [ ] **Step 6: SystemServicesSettings — move "Create Token" to header, remove Load/Refresh**

In `frontend/src/routes/settings/SystemServicesSettings.svelte`, replace lines 181-198 (the `<SectionCard>` opening tag plus the button `<div>`):

```svelte
<SectionCard title="System Enrollment Tokens">
	<div class="mb-4 flex items-center justify-between">
		<div class="flex gap-2">
			{#if tokens === null}
				<Button variant="primary" {loading} disabled={loading} onclick={() => void loadTokens(1)}>Load Tokens</Button>
			{:else}
				<Button variant="secondary" {loading} disabled={loading} onclick={() => void loadTokens(currentPage)}>
					Refresh
				</Button>
			{/if}
			<Button
				variant="primary"
				onclick={() => {
					showCreateDialog = true;
				}}>Create Token</Button
			>
		</div>
	</div>
```

with:

```svelte
<SectionCard title="System Enrollment Tokens">
	{#snippet actions()}
		<Button
			variant="primary"
			onclick={() => {
				showCreateDialog = true;
			}}>Create Token</Button
		>
	{/snippet}
```

- [ ] **Step 7: Run all affected tests**

```bash
cd frontend && npx vitest run --reporter=verbose src/routes/settings/EnrollmentTokenSettings.test.ts src/routes/settings/SystemServicesSettings.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: all tests pass. The "Revoke button" tests succeed without clicking "Load Tokens".

- [ ] **Step 8: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/routes/settings/OidcProvidersSettings.svelte \
        frontend/src/routes/settings/EnrollmentTokenSettings.svelte \
        frontend/src/routes/settings/SystemServicesSettings.svelte \
        frontend/src/routes/settings/EnrollmentTokenSettings.test.ts \
        frontend/src/routes/settings/SystemServicesSettings.test.ts
git commit -m "$(cat <<'EOF'
fix(frontend): move modal-trigger buttons to SectionCard header; remove Load/Refresh

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: SurfaceRenderer — Section Header Actions

> **Prerequisite:** Plan A must be merged. This task requires `header_action_ids?: InteractionId[]` in the `section` node variant of `frontend/src/lib/surfaces/contract.ts`.

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte:1-15,94-115`
- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts`

- [ ] **Step 1: Write failing test**

Add to `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts` inside `describe('SurfaceRenderer')`:

```typescript
it("renders section with header_action_ids as SectionCard with header button", async () => {
  const node: SurfaceNode = {
    kind: "section",
    title: "My Section",
    header_action_ids: ["create-item"],
    children: [],
  };
  const interactions: InteractionDescriptor[] = [
    {
      interaction_id: "create-item",
      kind: "mutation_action",
      label: "Create Item",
      transport: { mode: "controller_local" },
    },
  ];
  const { container } = render(SurfaceRenderer, {
    surfaceId: "test.surface",
    node,
    interactions,
  });
  expect(
    container.querySelector('[data-ui="section-card"]'),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Create Item" }),
  ).toBeInTheDocument();
});

it("renders section without header_action_ids as plain div (no SectionCard)", () => {
  const node: SurfaceNode = {
    kind: "section",
    title: "Plain Section",
    children: [],
  };
  const { container } = render(SurfaceRenderer, {
    surfaceId: "test.surface",
    node,
  });
  expect(
    container.querySelector('[data-ui="section-card"]'),
  ).not.toBeInTheDocument();
  expect(screen.getByText("Plain Section")).toBeInTheDocument();
});
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SurfaceRenderer.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓|section-card"
```

Expected: the header_action_ids test fails (no `[data-ui="section-card"]` rendered).

- [ ] **Step 3: Add imports to SurfaceRenderer.svelte**

In `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`, add two imports in the `<script>` block. Find the existing import group at the top and add:

```typescript
import SectionCard from "$lib/components/ui/SectionCard.svelte";
import SurfaceInteractionButton from "./SurfaceInteractionButton.svelte";
```

- [ ] **Step 4: Add notifySurfaceReload helper**

After the `findInteraction`/`findDataSource` helpers (around line 67-73), add:

```typescript
function notifySurfaceReload(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent("surface:reload", {
      detail: { surfaceId, targetProviderId: targetProviderId ?? null },
    }),
  );
}
```

- [ ] **Step 5: Update section branch in template**

Replace the existing section branch (lines 94-115):

```svelte
{#if node.kind === 'section'}
	<div class="space-y-4">
		{#if node.title}
			<h3 class="text-subsection-title font-bold text-[var(--text-primary)]">{node.title}</h3>
		{/if}
		{#each node.children ?? [] as child, idx (idx)}
			<SurfaceRenderer
				{surfaceId}
				node={child}
				{interactions}
				{dataSources}
				{targetProviderId}
				{encryptionContext}
				{dataBySource}
				{baseParams}
				{requiredContextParam}
				{requiredForInteractionIds}
				{pageBySource}
				{onPageChange}
			/>
		{/each}
	</div>
```

with:

```svelte
{#if node.kind === 'section'}
	{#if (node.header_action_ids ?? []).length > 0}
		<SectionCard title={node.title}>
			{#snippet actions()}
				{#each node.header_action_ids ?? [] as actionId (actionId)}
					{@const headerInteraction = findInteraction(actionId)}
					{#if headerInteraction}
						<SurfaceInteractionButton
							{surfaceId}
							interaction={headerInteraction}
							{interactions}
							{targetProviderId}
							{encryptionContext}
							{baseParams}
							size="sm"
							oncomplete={async () => {
								notifySurfaceReload();
							}}
						/>
					{/if}
				{/each}
			{/snippet}
			{#each node.children ?? [] as child, idx (idx)}
				<SurfaceRenderer
					{surfaceId}
					node={child}
					{interactions}
					{dataSources}
					{targetProviderId}
					{encryptionContext}
					{dataBySource}
					{baseParams}
					{requiredContextParam}
					{requiredForInteractionIds}
					{pageBySource}
					{onPageChange}
				/>
			{/each}
		</SectionCard>
	{:else}
		<div class="space-y-4">
			{#if node.title}
				<h3 class="text-subsection-title font-bold text-[var(--text-primary)]">{node.title}</h3>
			{/if}
			{#each node.children ?? [] as child, idx (idx)}
				<SurfaceRenderer
					{surfaceId}
					node={child}
					{interactions}
					{dataSources}
					{targetProviderId}
					{encryptionContext}
					{dataBySource}
					{baseParams}
					{requiredContextParam}
					{requiredForInteractionIds}
					{pageBySource}
					{onPageChange}
				/>
			{/each}
		</div>
	{/if}
```

- [ ] **Step 6: Run tests — verify they pass**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SurfaceRenderer.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: all tests pass including the two new section tests.

- [ ] **Step 7: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceRenderer.svelte \
        frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts
git commit -m "$(cat <<'EOF'
feat(frontend): render section header_action_ids in SurfaceRenderer via SectionCard

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: SchemaForm Draft Mode

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SchemaForm.svelte`
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.svelte`

Draft state is managed by `createFormDraft<FieldRecord>` from `$lib/forms/draft.svelte` — do NOT re-implement dirty tracking inline. `form.draft` is the reactive value store; `form.isDirty` and `form.isFieldDirty(key)` drive UI state; `form.load()` sets the server baseline; `form.commit()` advances the baseline after a successful save; `form.discard()` reverts to the baseline. When `loadInitialValues` is absent (create mode), `draftMode = false` — Save is enabled whenever the form is valid, no Discard button. When `loadInitialValues` is present (edit mode), `draftMode = true` — Save is disabled until dirty, Discard appears when dirty.

Multi-select fields keep a parallel `multiSets` (`Record<string, SvelteSet<string>>`) for rendering. The draft stores the sorted NUL-joined string (`setToDraftString(set)`). `draftStringToSet(s)` reconstructs the `SvelteSet` when needed.

- [ ] **Step 1: Write failing tests**

Add to `frontend/src/lib/components/surfaces/SchemaForm.test.ts`:

```typescript
describe("SchemaForm draft mode", () => {
  it("in create mode (no loadInitialValues), Save is enabled when valid", async () => {
    render(SchemaForm, {
      fields: [
        { key: "name", label: "Name", field_type: "text", required: true },
      ],
      onsubmit: vi.fn().mockResolvedValue(undefined),
    });
    // required field empty → invalid → disabled
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await fireEvent.input(screen.getByLabelText(/Name/i), {
      target: { value: "Hello" },
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled(),
    );
    // no Discard in create mode
    expect(
      screen.queryByRole("button", { name: "Discard" }),
    ).not.toBeInTheDocument();
  });

  it("in edit mode, Save is disabled when values match server baseline", async () => {
    const loadInitialValues = vi.fn().mockResolvedValue({ name: "Original" });
    render(SchemaForm, {
      fields: [
        { key: "name", label: "Name", field_type: "text", required: true },
      ],
      onsubmit: vi.fn().mockResolvedValue(undefined),
      loadInitialValues,
    });
    await waitFor(() =>
      expect(screen.getByDisplayValue("Original")).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "Discard" }),
    ).not.toBeInTheDocument();
  });

  it("in edit mode, Save enabled and Discard visible after change", async () => {
    const loadInitialValues = vi.fn().mockResolvedValue({ name: "Original" });
    render(SchemaForm, {
      fields: [
        { key: "name", label: "Name", field_type: "text", required: true },
      ],
      onsubmit: vi.fn().mockResolvedValue(undefined),
      loadInitialValues,
    });
    await waitFor(() => screen.getByDisplayValue("Original"));
    await fireEvent.input(screen.getByDisplayValue("Original"), {
      target: { value: "Changed" },
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled(),
    );
    expect(screen.getByRole("button", { name: "Discard" })).toBeInTheDocument();
  });

  it("Discard restores server values and hides itself", async () => {
    const loadInitialValues = vi.fn().mockResolvedValue({ name: "Original" });
    render(SchemaForm, {
      fields: [
        { key: "name", label: "Name", field_type: "text", required: true },
      ],
      onsubmit: vi.fn().mockResolvedValue(undefined),
      loadInitialValues,
    });
    await waitFor(() => screen.getByDisplayValue("Original"));
    await fireEvent.input(screen.getByDisplayValue("Original"), {
      target: { value: "Changed" },
    });
    const discardBtn = await screen.findByRole("button", { name: "Discard" });
    await fireEvent.click(discardBtn);
    await waitFor(() =>
      expect(screen.getByDisplayValue("Original")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: "Discard" }),
    ).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SchemaForm.test.ts -t "draft mode" 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: all four draft-mode tests fail.

- [ ] **Step 3: Replace `values` state with `createFormDraft`, add helpers**

In `frontend/src/lib/components/surfaces/SchemaForm.svelte`:

**3a. Add import** (at the top of `<script>`):

```typescript
import { createFormDraft } from "$lib/forms/draft.svelte";
type FieldRecord = Record<string, unknown>;
```

**3b. Replace `let values: Record<string, string> = $state({})` with:**

```typescript
const form = createFormDraft<FieldRecord>({});
```

Keep `let multiSets: Record<string, SvelteSet<string>> = $state({})` as-is (rendering only).

**3c. Update the existing `fieldValue` helper (line 46) — do NOT add a new one:**

```typescript
// Before:
function fieldValue(key: string): string {
  return values[key] ?? '';
}

// After:
function fieldValue(key: string): string {
  return (form.draft[key] as string) ?? '';
}
```

Then add the remaining helpers immediately after `fieldValue`:

```typescript
function setToDraftString(set: SvelteSet<string>): string {
  return [...set].sort().join("\0");
}
function draftStringToSet(s: string): SvelteSet<string> {
  return new SvelteSet(s ? s.split("\0") : []);
}
// Converts a server response to a draft-compatible record.
// Iterates ALL form fields so that optional keys absent in `raw` get a '' sentinel —
// this ensures isDirty detects edits to fields the server omitted.
function normalizeForDraft(raw: Record<string, unknown>): FieldRecord {
  const result: FieldRecord = {};
  for (const f of fields) {
    const v = raw[f.key];
    result[f.key] = Array.isArray(v)
      ? [...(v as string[])].sort().join("\0")
      : v === null || v === undefined
        ? ""
        : String(v);
  }
  return result;
}
function handleDiscard() {
  form.discard();
  for (const f of fields) {
    if (f.field_type === "multi_select") {
      multiSets[f.key] = draftStringToSet((form.draft[f.key] as string) ?? "");
    }
  }
}
const draftMode = $derived(loadInitialValues !== undefined);
// Reactive validity — drives Save disabled state before first submit attempt.
const isValid = $derived(fields.every((f) => validateField(f) === null));
```

**3d. Update all reads and writes of `values[key]` throughout the component:**

- Every `values[key]` read in functions/handlers → `fieldValue(key)` (already handled for `validateField` since `fieldValue` now reads from `form.draft`)
- Every `values[key] = x` write in event handlers (lines ~368, 390, 453, 474) → `form.update(key, x)`
- **Line 408 `bind:value={values[field.key]}`** (Select) is a two-way binding — replace with controlled pattern:

  ```svelte
  <!-- Before: -->
  <Select bind:value={values[field.key]} ...>

  <!-- After: -->
  <Select
    value={fieldValue(field.key)}
    onchange={(v: string) => { form.update(field.key, v); }}
    ...>
  ```

  Verify the exact prop names against the `Select` component signature — it may use `onChange` (camelCase).

- **Line 481 `bind:value={values[field.key]}`** (hidden input) — replace with one-way:

  ```svelte
  <input type="hidden" name={field.key} value={fieldValue(field.key)} />
  ```

- Multi-select onchange handlers (wherever `multiSets[key]` is toggled) → after updating the set, call `form.update(field.key, setToDraftString(multiSets[field.key]))`
- **`validateField()` internal reads** (lines ~111 toggle check, ~113 text check) — both read `values[field.key]` directly, not via `fieldValue()`. Update those lines too: `values[field.key]` → `fieldValue(field.key)`.
- **`isFieldVisible()` at line ~304** — reads `values[field.visible_when.field]`. Update to `fieldValue(field.visible_when.field)`.

- [ ] **Step 4: Update `$effect` to use `form.load()` instead of `values =`**

In `frontend/src/lib/components/surfaces/SchemaForm.svelte`, in the `$effect` block that loads initial values:

**Replace all `values = initial` / `values = loadedValues` assignments** with `form.load(normalizeForDraft(initial))` / `form.load(normalizeForDraft(obj))`.

**After each `form.load()` call**, rebuild `multiSets` from the loaded draft:

```typescript
// After form.load(normalizeForDraft(obj)):
const nextMulti: Record<string, SvelteSet<string>> = {};
for (const f of fields) {
  if (f.field_type === "multi_select") {
    nextMulti[f.key] = draftStringToSet((form.draft[f.key] as string) ?? "");
  }
}
multiSets = nextMulti;
```

Remove any `serverValues`/`serverMultiValues` assignments that may already exist from a previous partial edit.

- [ ] **Step 5: Update `handleSubmit` + `SurfaceForm.svelte` for `form.commit()`**

**5a. SchemaForm.svelte — change `onsubmit` prop type from `Promise<void>` to `Promise<unknown>`.**

In the component props, the `onsubmit` function must return `Promise<unknown>` so SchemaForm can receive the server response and pass it to `form.commit()`.

**5b. In `handleSubmit`, capture the result and commit or reload:**

```typescript
const result = await onsubmit(payload); // now Promise<unknown>
if (draftMode) {
  if (result !== null && typeof result === "object" && !Array.isArray(result)) {
    form.commit(normalizeForDraft(result as Record<string, unknown>));
  } else {
    // server returned non-object (e.g. void); reload from server
    const reloaded = await loadInitialValues!();
    form.load(normalizeForDraft(reloaded));
  }
  // rebuild multiSets after commit/reload
  const nextMulti: Record<string, SvelteSet<string>> = {};
  for (const f of fields) {
    if (f.field_type === "multi_select") {
      nextMulti[f.key] = draftStringToSet((form.draft[f.key] as string) ?? "");
    }
  }
  multiSets = nextMulti;
}
```

**5c. SurfaceForm.svelte — make `submitInteraction` return `Promise<unknown>`:**

In `frontend/src/lib/components/surfaces/SurfaceForm.svelte`, the `submitInteraction` function currently discards the `invokeSurfaceInteraction` result. Keep the full try/catch/finally and all side effects (`showSuccess`, `oncomplete`). The only change is to capture and return the result:

```typescript
async function submitInteraction(
  params: Record<string, unknown>,
): Promise<unknown> {
  submitting = true;
  try {
    const request = await buildSurfaceInteractionRequest(interaction, params, {
      targetProviderId,
      encryption: encryptionContext,
    });
    const result = await invokeSurfaceInteraction(
      surfaceId,
      interaction.interaction_id,
      request,
    );
    showSuccess(`${actionLabel} completed`);
    await oncomplete?.(result);
    return result; // NEW: return so SchemaForm can call form.commit(result)
  } catch (error) {
    showError(error instanceof Error ? error.message : "Interaction failed");
    throw error; // re-throw so SchemaForm handleSubmit catches it and skips commit
  } finally {
    submitting = false;
  }
}
```

Also update the call in `handleSubmit` where `await submitInteraction(mergedParams)` is used inside a non-schema form path — add `return` or discard the result (it is not used there).

- [ ] **Step 6: Add `dirty={form.isFieldDirty(field.key)}` to all FormFieldRow calls**

In the template section of `frontend/src/lib/components/surfaces/SchemaForm.svelte`, every `<FormFieldRow>` already accepts a `dirty` prop (it renders a left accent border when truthy). Add `dirty={form.isFieldDirty(field.key)}` to all six FormFieldRow variants:

The textarea branch FormFieldRow:

```svelte
<FormFieldRow
  label={field.label}
  inputId={field.key}
  required={field.required}
  hint={field.help_text}
  error={fieldErrors[field.key]}
  dirty={form.isFieldDirty(field.key)}
>
```

The ssh_private_key branch FormFieldRow — same change.

The select branch FormFieldRow — same change.

The multi_select branch FormFieldRow (this one has no `inputId` currently):

```svelte
<FormFieldRow
  label={field.label}
  required={field.required}
  hint={field.help_text}
  error={fieldErrors[field.key]}
  dirty={form.isFieldDirty(field.key)}
>
```

The toggle branch FormFieldRow — same change (add `dirty={form.isFieldDirty(field.key)}`).

The default text/input branch FormFieldRow — same change.

- [ ] **Step 7: Update submit button area**

Replace the existing submit button block at the bottom of the template (lines ~485-489):

```svelte
{#if !hideSubmit}
	<Button type="submit" variant="primary" loading={loading || preLoading}>
		{submitLabel}
	</Button>
{/if}
```

with:

```svelte
{#if !hideSubmit}
	<div class="flex gap-2 justify-end">
		{#if draftMode && form.isDirty}
			<Button type="button" variant="ghost" disabled={loading || preLoading} onclick={handleDiscard}>
				Discard
			</Button>
		{/if}
		<Button
			type="submit"
			variant="primary"
			loading={loading || preLoading}
			disabled={!isValid || loading || preLoading || (draftMode && !form.isDirty)}
		>
			{submitLabel}
		</Button>
	</div>
{/if}
```

- [ ] **Step 8: Run draft-mode tests**

```bash
cd frontend && npx vitest run --reporter=verbose src/lib/components/surfaces/SchemaForm.test.ts 2>&1 | grep -E "FAIL|PASS|×|✓"
```

Expected: all tests pass, including the four new draft-mode tests and existing option-loading tests.

- [ ] **Step 9: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add frontend/src/lib/components/surfaces/SchemaForm.svelte \
        frontend/src/lib/components/surfaces/SchemaForm.test.ts
git commit -m "$(cat <<'EOF'
feat(frontend): add draft mode to SchemaForm with dirty tracking and Discard button

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Design Language Documentation

**Files:**

- Modify: `docs/development/ui/surfaces.md`
- Modify: `docs/development/ui/primitives.md`

- [ ] **Step 1: Update surfaces.md — Section primitive row**

In `docs/development/ui/surfaces.md`, the Surface Primitives table currently has:

```markdown
| `Section` | Vertical stack with `16px` gap |
```

Replace with:

```markdown
| `Section` | Vertical stack with `16px` gap; use `header_action_ids` to add modal/workflow trigger buttons to the card header (see [Section Header Actions](#section-header-actions)) |
```

- [ ] **Step 1b: Update surfaces.md — ActionBar primitive row**

Find the `ActionBar` row in the Surface Primitives table and add the ordering rule:

```markdown
| `ActionBar` | Row of primary action buttons for the section; must appear **before** any `Table` sibling in the parent `Section`'s children list — buttons above data |
```

- [ ] **Step 2: Update surfaces.md — Form primitive row (add draft behavior)**

Find the Form row in the Surface Primitives table:

```markdown
| `Form` | Same `FormFieldRow` + `Input`/`Textarea`/`Checkbox` layout as built-in forms |
```

Replace with:

```markdown
| `Form` | Same `FormFieldRow` + `Input`/`Textarea`/`Checkbox` layout as built-in forms; when `pre_load_interaction` is set, draft mode activates: Save disabled until dirty, Discard visible when dirty, dirty fields get left-accent highlight (see [Form Draft Mode](#form-draft-mode)) |
```

- [ ] **Step 3: Add Section Header Actions section to surfaces.md**

After the Surface Primitives table (before the Context Selector section), add:

````markdown
## Section Header Actions

**Status:** `Implemented`

A `Section` node can declare action buttons that render in the `SectionCard` header row, to the right of the section title. This replaces the anti-pattern of placing modal-trigger buttons inside the section body.

### Contract

```typescript
// frontend/src/lib/surfaces/contract.ts
{
  kind: 'section';
  title?: string;
  header_action_ids?: InteractionId[];  // ≤ 3 entries; each must resolve to modal or workflow trigger
  children?: SurfaceNode[];
}
```

### Rust builder

```rust
// Use the typed constructor — SurfaceNode::Section is #[non_exhaustive]
SurfaceNode::section_with_header_actions(
    Some("My Section"),
    vec![interaction_id!("create-item")],
    vec![/* children */],
)
```

### Constraints

- Maximum **3** entries in `header_action_ids`. Registration fails with `SchemaOrLimitFailure` if exceeded.
- Every ID in `header_action_ids` must reference an interaction whose kind is `ModalTrigger` or `WorkflowTrigger`. Any other kind causes a `SchemaOrLimitFailure` rejection at registration time.
- Frontend skips IDs that cannot be resolved to a known interaction (graceful degradation).
- When `header_action_ids` is present and non-empty, `SurfaceRenderer` wraps the section in a `SectionCard`. When absent or empty, the plain `<div class="space-y-4">` layout is used — no `SectionCard` chrome.

### When to use

Use `header_action_ids` for actions that **open a modal or start a workflow** — creating a new record, running a wizard. Do **not** use it for form save/discard buttons; those belong in the section body with `flex gap-2 justify-end`.
````

- [ ] **Step 4: Add Form Draft Mode section to surfaces.md**

After the Section Header Actions section, add:

```markdown
## Form Draft Mode

**Status:** `Implemented`

Surface forms backed by `pre_load_interaction` automatically enter draft mode. The form fetches the current server values on mount, then tracks dirty state field-by-field.

### Behavior

| Condition                                            | Save button        | Discard button |
| ---------------------------------------------------- | ------------------ | -------------- |
| `pre_load_interaction` absent (create mode), valid   | Enabled            | Hidden         |
| `pre_load_interaction` absent (create mode), invalid | Disabled           | Hidden         |
| Edit mode, values match server baseline              | Disabled           | Hidden         |
| Edit mode, at least one field changed                | Enabled            | Visible        |
| Submitting or loading initial values                 | Disabled (spinner) | Hidden         |

- **Dirty fields** receive a left-side accent border (the `dirty` prop on `FormFieldRow`).
- **Discard** restores all fields to the last server-fetched values without a network round-trip.
- **Save** commits the current values as the new baseline on success — no reload needed to re-enable Save for subsequent edits.

### Caveats

- The JSON-payload fallback (no `form_ui` / `fields`) is **not** in draft mode. It remains stateless.
- Multi-select dirty detection uses sorted NUL-joined string comparison. Field order from the server does not affect dirty state.
```

- [ ] **Step 4b: Add Section Layout Rules section to surfaces.md**

After the Section Header Actions section (before Form Draft Mode), add:

````markdown
## Section Layout Rules

**Rule:** `ActionBar` nodes must appear **before** any `Table` node in the same parent `Section`'s children list. This ensures action buttons render above the data table in the UI.

**Correct:**

```rust
SurfaceNode::section(None, vec![
    SurfaceNode::ActionBar { action_ids: primary_ids },
    SurfaceNode::Table { … },
])
```

**Incorrect (do not do this):**

```rust
// Wrong — table renders before buttons
SurfaceNode::section(None, vec![
    SurfaceNode::Table { … },
    SurfaceNode::ActionBar { action_ids: primary_ids },
])
```

This ordering applies to all surface providers (built-in, plugin, service). There is no runtime enforcement; it is a contract convention checked by code review.
````

- [ ] **Step 5: Update primitives.md — SectionCard button placement rules**

In `docs/development/ui/primitives.md`, after the existing SectionCard usage example (after the block that ends `...render the card without a header.`), add:

````markdown
### SectionCard button placement rules

Two distinct button categories exist in `SectionCard` — they go in different locations:

| Button category                                                | Location                                                        | Example                              |
| -------------------------------------------------------------- | --------------------------------------------------------------- | ------------------------------------ |
| Modal/workflow trigger (opens a new dialog or starts a wizard) | `{#snippet actions()}` slot in the card header                  | "Add Provider", "Create Token"       |
| Form action (saves or discards the card's own form)            | Card body, `<div class="flex gap-2 justify-end">` at the bottom | Save, Discard, Reset Data, Rotate CA |

**Modal/workflow triggers in the header:**

```svelte
<SectionCard title="OIDC Providers">
  {#snippet actions()}
    <Button variant="primary" onclick={openCreate}>Add Provider</Button>
  {/snippet}
  <!-- table or body content -->
</SectionCard>
```

**Form actions right-aligned in the body:**

```svelte
<SectionCard title="Network Settings">
  <!-- form fields -->
  <div class="flex gap-2 justify-end">
    {#if form.isDirty}
      <Button variant="ghost" onclick={() => form.discard()}>Discard</Button>
    {/if}
    <Button variant="primary" disabled={!form.isDirty} onclick={save}>Save</Button>
  </div>
</SectionCard>
```

Confirmation-dialog triggers (buttons that open a `ConfirmDialog`, not a `ModalShell`) are **not** modal-triggers. They stay in the card body and must be right-aligned: `<div class="flex justify-end">`.

Titles in `SectionCard` must follow **Title Case** (e.g. "OAuth Settings", not "OAuth settings").
````

- [ ] **Step 6: Update primitives.md — createFormDraft example**

Find the existing `createFormDraft` usage example in `docs/development/ui/primitives.md`. Update the button row in the example to use `flex gap-2 justify-end`, show Discard only when dirty, label Save always as "Save", and document the dirty field prop:

````markdown
**Form action button rules (applies to all hand-written forms using `createFormDraft`):**

```svelte
<div class="flex gap-2 justify-end">
  {#if form.isDirty}
    <Button variant="ghost" onclick={() => form.discard()}>Discard</Button>
  {/if}
  <!-- Save is always visible; disabled when not dirty or invalid -->
  <Button variant="primary" disabled={!form.isDirty || !isValid} onclick={save}>Save</Button>
</div>
```

- The submit label is always **"Save"** for form-save actions.
- Pass `dirty={form.isFieldDirty(key)}` to each `FormFieldRow` to show the left-accent dirty indicator.
````

- [ ] **Step 7: Run markdownlint**

```bash
npx markdownlint --config .markdownlint.json docs/development/ui/surfaces.md docs/development/ui/primitives.md 2>&1
```

Expected: no errors. If any, fix line-length or heading issues and re-run.

- [ ] **Step 8: Commit**

```bash
git add docs/development/ui/surfaces.md docs/development/ui/primitives.md
git commit -m "$(cat <<'EOF'
docs(ui): document section header actions, form draft mode, and button placement rules

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Modal Footer Submit Label Unification (R9)

**Files:**

- Modify: `frontend/src/routes/settings/OidcProvidersSettings.svelte`
- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`
- Modify: `frontend/src/routes/settings/SystemServicesSettings.svelte`

These three components have inline modals (not SchemaForm) with hardcoded "Create" or "Update" labels. Per spec R9, all form-save actions must read "Save".

- [ ] **Step 1: OidcProvidersSettings — change modal footer button labels**

In `frontend/src/routes/settings/OidcProvidersSettings.svelte`, find the modal footer that renders when `showOidcModal` is true. There are two button label paths:

**Create path** — find the `<Button variant="primary" ...>Create</Button>` (shown when `editingProvider === null`) and change to:

```svelte
<Button variant="primary" loading={saving} onclick={handleSaveOidc}>Save</Button>
```

**Edit path** — find the `<Button variant="primary" ...>Update</Button>` (shown when `editingProvider !== null`) and change to:

```svelte
<Button variant="primary" loading={saving} onclick={handleSaveOidc}>Save</Button>
```

If both paths use a single button whose label is conditional (e.g. `{editingProvider ? 'Update' : 'Create'}`), replace the entire expression with `Save`.

- [ ] **Step 2: EnrollmentTokenSettings — change modal footer button label**

In `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`, find the ModalShell footer and change:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Create</Button>
```

to:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Save</Button>
```

- [ ] **Step 3: SystemServicesSettings — change modal footer button label**

In `frontend/src/routes/settings/SystemServicesSettings.svelte` line ~271, find the ModalShell footer and change:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Create</Button>
```

to:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Save</Button>
```

- [ ] **Step 4: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | grep -E "Error|error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/settings/OidcProvidersSettings.svelte \
        frontend/src/routes/settings/EnrollmentTokenSettings.svelte \
        frontend/src/routes/settings/SystemServicesSettings.svelte
git commit -m "$(cat <<'EOF'
fix(frontend): unify modal footer submit labels to 'Save'

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Final Quality Gate

- [ ] **Step 1: Full frontend check + test suite**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test 2>&1 | tail -30
```

Expected: all lint, type-check, and test suite pass with no errors.

- [ ] **Step 2: Markdown lint on all touched docs**

```bash
npx markdownlint --config .markdownlint.json docs/development/ui/surfaces.md docs/development/ui/primitives.md 2>&1
```

Expected: no errors.

- [ ] **Step 3: Grep confirm no 'Submit' default remains in surface form files**

```bash
grep -n "= 'Submit'" frontend/src/lib/components/surfaces/SurfaceForm.svelte frontend/src/lib/components/surfaces/SchemaForm.svelte
```

Expected: no output (all 'Submit' defaults replaced with 'Save').

- [ ] **Step 4: Grep confirm all Load/Refresh buttons removed**

```bash
grep -n "Load Tokens\|Refresh" frontend/src/routes/settings/EnrollmentTokenSettings.svelte frontend/src/routes/settings/SystemServicesSettings.svelte
```

Expected: no output.

- [ ] **Step 5: Grep confirm button rows are right-aligned**

```bash
grep -n "flex gap-2\"" frontend/src/routes/settings/AgentCertificateSettings.svelte frontend/src/routes/settings/McpAccessTab.svelte frontend/src/routes/settings/GlobalSettingsTab.svelte
```

Expected: no output (all `flex gap-2` without `justify-end` eliminated in action-button rows). If any hit is a non-action-button row (e.g. inline icon+text groups), verify visually.

- [ ] **Step 6: Grep confirm no 'Create'/'Update'/'Submit' labels remain in settings modal footers (R9)**

```bash
grep -rn '"Create"\|"Update"\|"Submit"' frontend/src/routes/settings/
```

Expected: no output matching modal footer button labels. Matches inside variable names or comments are false positives — inspect the context of any remaining hit before marking done.

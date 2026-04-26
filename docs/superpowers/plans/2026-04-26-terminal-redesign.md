# Terminal Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL:
> Use `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the terminal shell’s stacked callouts with a single critical banner,
compact footer badges, collapsed details, and deterministic empty-state behavior
everywhere the shared terminal is used.

**Architecture:** Keep the redesign centered in the shared `TerminalOutput`
component and make route code map backend state into an explicit terminal
contract. History and software pages should stop inventing terminal presentation
with `callouts[]` and instead pass `criticalBanner`, `inlineBadges`, `details`,
and `emptyState` into the shared shell.

**Tech Stack:** Svelte 5, TypeScript, Vitest, Testing Library, xterm.js

---

## File Structure

- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
  Responsibility: shared terminal contract, banner/badge/details/empty-state
  rendering, removal of in-shell `Callout` usage.
- Modify: `frontend/src/lib/components/TerminalOutput.test.ts`
  Responsibility: unit coverage for the new shared terminal contract and
  shell-level precedence behavior.
- Modify: `frontend/src/routes/history/+page.svelte`
  Responsibility: map `UpdateHistoryResponse` into the new shared terminal props
  for history-driven terminal sessions.
- Modify: `frontend/src/routes/history/history.test.ts`
  Responsibility: route-level assertions for
  waiting/truncated/details/interactive history terminals after the redesign.
- Modify: `frontend/src/routes/history/history-trigger-status.test.ts`
  Responsibility: route-level assertions for collapsed details behavior in
  modal-trigger flows.
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
  Responsibility: map live software-update modal state into the new shared
  terminal props.
- Modify: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`
  Responsibility: route-level assertions that software-triggered terminals use
  compact badges instead of route-local warning status treatment.

## Task 1: Refactor the Shared Terminal Contract

**Files:**

- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
- Test: `frontend/src/lib/components/TerminalOutput.test.ts`

- [ ] **Step 1: Write the failing shared-terminal tests**

Add tests that lock the approved contract into place before changing the component:

```ts
it('renders a single critical banner without using Callout markup', async () => {
	render(TerminalOutput as never, {
		open: true,
		title: 'Demo App on host-one',
		statusLabel: 'Queued',
		statusTone: 'warning',
		metadata: 'host-one · started just now · 0m',
		criticalBanner: {
			tone: 'warning',
			label: 'Output truncated',
			message: 'Only the first 50 MB is stored.'
		},
		onclose: vi.fn()
	} as never);

	expect(screen.getByText('Output truncated')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-critical-banner"]')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-shell"] [data-ui="callout"]')).not.toBeInTheDocument();
});

it('keeps details collapsed until explicitly opened', async () => {
	render(TerminalOutput as never, {
		open: true,
		title: 'Demo App on host-one',
		statusLabel: 'Completed',
		statusTone: 'success',
		metadata: 'host-one · started just now · 0m',
		details: [
			{ id: 'actor', label: 'Actor', value: 'user (actor-1)' },
			{ id: 'recovery', label: 'Recovery hint', value: 'Retry after fixing permissions.' }
		],
		onclose: vi.fn()
	} as never);

	expect(screen.queryByText('user (actor-1)')).not.toBeInTheDocument();
	await fireEvent.click(screen.getByRole('button', { name: /details/i }));
	expect(screen.getByText('user (actor-1)')).toBeInTheDocument();
	expect(screen.getByText('Retry after fixing permissions.')).toBeInTheDocument();
});

it('renders an empty state without mounting xterm when there is no live session and no output', async () => {
	render(TerminalOutput as never, {
		open: true,
		title: 'Demo App on host-one',
		statusLabel: 'Queued',
		statusTone: 'warning',
		metadata: 'host-one · started just now · 0m',
		showTerminal: false,
		emptyState: {
			label: 'Queued',
			message: 'Waiting for another update on this host to finish.'
		},
		onclose: vi.fn()
	} as never);

	expect(screen.getByText('Waiting for another update on this host to finish.')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-empty-state"]')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-output"]')).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the shared-terminal unit test file and verify it fails**

Run:

```bash
npm --prefix frontend test -- src/lib/components/TerminalOutput.test.ts
```

Expected: FAIL because `TerminalOutput.svelte` still imports `Callout`,
exposes `callouts`, and has no `criticalBanner`, `inlineBadges`, `details`, or
`emptyState` rendering.

- [ ] **Step 3: Implement the minimal shared terminal redesign**

Replace the `callouts` contract with explicit terminal-specific types and render the new shell structure. Keep xterm behavior intact.

```svelte
<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { StatusBadge } from '$lib/components/ui';

	type TerminalBanner = {
		tone: 'warning' | 'danger' | 'info';
		label: string;
		message: string;
	};

	type TerminalInlineBadge = {
		id: string;
		tone: 'neutral' | 'info' | 'success' | 'warning' | 'danger';
		label: string;
	};

	type TerminalDetail = {
		id: string;
		label: string;
		value: string;
	};

	type TerminalEmptyState = {
		label?: string;
		message: string;
	};

	interface Props {
		criticalBanner?: TerminalBanner;
		inlineBadges?: TerminalInlineBadge[];
		details?: TerminalDetail[];
		emptyState?: TerminalEmptyState;
		// keep existing title/status/metadata/output/onInput/actions/showTerminal props
	}

	let {
		criticalBanner,
		inlineBadges = [],
		details = [],
		emptyState,
		actions = [],
		// existing props...
	} = $props();

	let detailsOpen = $state(false);
</script>

<div class="terminal-body" data-ui="terminal-body">
	{#if criticalBanner}
		<div class="terminal-critical-banner" data-ui="terminal-critical-banner" data-tone={criticalBanner.tone}>
			<strong>{criticalBanner.label}</strong>
			<span>{criticalBanner.message}</span>
		</div>
	{/if}

	{#if showTerminal}
		<div bind:this={containerEl} class="terminal-output" data-ui="terminal-output"></div>
	{:else if emptyState}
		<div class="terminal-empty-state" data-ui="terminal-empty-state">
			{#if emptyState.label}<span class="terminal-empty-state-label">{emptyState.label}</span>{/if}
			<p>{emptyState.message}</p>
		</div>
	{/if}
</div>

<footer class="terminal-statusbar" data-ui="terminal-statusbar">
	<div class="terminal-status-leading">
		<StatusBadge tone={statusTone} label={statusLabel} />
		{#if inlineBadges.length > 0}
			<div class="terminal-inline-badges" data-ui="terminal-inline-badges">
				{#each inlineBadges as badge (badge.id)}
					<StatusBadge tone={badge.tone} label={badge.label} />
				{/each}
			</div>
		{/if}
	</div>
	<div class="terminal-status-trailing">
		{#if actions.length > 0}
			<div class="terminal-actions" data-ui="terminal-actions">
				{#each actions as action (action.id ?? action.label)}
					<button
						type="button"
						class={`terminal-action terminal-action--${action.tone ?? 'neutral'}`}
						title={action.title}
						disabled={action.disabled}
						onclick={action.onclick}
					>
						{action.label}
					</button>
				{/each}
			</div>
		{/if}
		{#if details.length > 0}
			<div class="terminal-details">
				<button type="button" data-ui="terminal-details-toggle" onclick={() => (detailsOpen = !detailsOpen)}>
					Details
				</button>
				{#if detailsOpen}
					<div class="terminal-details-panel" data-ui="terminal-details-panel">
						{#each details as detail (detail.id)}
							<div data-ui="terminal-detail-row">
								<strong>{detail.label}</strong>
								<span>{detail.value}</span>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
		<span class="terminal-metadata">{metadata}</span>
	</div>
</footer>
```

- [ ] **Step 4: Run the shared-terminal unit test file again**

Run:

```bash
npm --prefix frontend test -- src/lib/components/TerminalOutput.test.ts
```

Expected: PASS with coverage for critical banner rendering, no in-shell `Callout`, collapsed details, and empty-state gating.

- [ ] **Step 5: Commit the shared terminal refactor**

```bash
git add frontend/src/lib/components/TerminalOutput.svelte frontend/src/lib/components/TerminalOutput.test.ts
git commit -m "feat(frontend): redesign shared terminal shell"
```

## Task 2: Update History Route Mapping to the New Terminal Contract

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte`
- Test: `frontend/src/routes/history/history.test.ts`
- Test: `frontend/src/routes/history/history-trigger-status.test.ts`

- [ ] **Step 1: Write the failing history-route tests**

Replace callout-oriented assertions with the new shell behavior:

```ts
it('shows truncation as the single critical banner and keeps actor/details collapsed by default', async () => {
	render(HistoryPage);
	await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

	const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
	await fireEvent.click(nginxEntry.querySelector('button[aria-expanded="false"]') as HTMLElement);

	expect(await screen.findByText('Output truncated')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-critical-banner"]')).toBeInTheDocument();
	expect(screen.queryByText('user (actor-1)')).not.toBeInTheDocument();
	expect(screen.getByRole('button', { name: /details/i })).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-shell"] [data-ui="callout"]')).not.toBeInTheDocument();
});

it('keeps waiting-state output lightweight without mounting terminal output', async () => {
	render(HistoryPage);
	await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

	const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
	await fireEvent.click(nginxEntry.querySelector('button[aria-expanded="false"]') as HTMLElement);

	expect(await screen.findByText(/waiting for another update/i)).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-empty-state"]')).toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-output"]')).not.toBeInTheDocument();
});

it('reveals recovery and protection details only after opening Details', async () => {
	vi.mocked(api.listUpdateHistory).mockResolvedValue(
		makeHistoryPage([
			makeHistoryEntry({
				pre_update_protection_summary: 'Pre-update checks blocked this run.',
				recovery_hint: 'Resolve the reported issue, then retry the update.'
			})
		])
	);

	render(HistoryPage);
	await waitFor(() => expect(screen.getByText('Demo App on Host One')).toBeInTheDocument());
	await fireEvent.click(screen.getByRole('button', { name: /view logs/i }));

	expect(screen.queryByText('Pre-update checks blocked this run.')).not.toBeInTheDocument();
	await fireEvent.click(screen.getByRole('button', { name: /details/i }));
	expect(screen.getByText('Pre-update checks blocked this run.')).toBeInTheDocument();
	expect(screen.getByText('Resolve the reported issue, then retry the update.')).toBeInTheDocument();
});
```

- [ ] **Step 2: Run the history-route tests and verify they fail**

Run:

```bash
npm --prefix frontend test -- src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts
```

Expected: FAIL because the history route still builds `callouts[]`, still
exposes actor/details immediately, and still treats waiting states as
callout-only terminal content.

- [ ] **Step 3: Implement the history-route mapping helpers**

Replace `terminalCalloutsFor()` with explicit helpers that match the shared component API and the spec’s precedence rules.

```ts
function terminalCriticalBannerFor(item: UpdateHistoryResponse) {
	if (!item.output_truncated) return undefined;
	return {
		tone: 'warning' as const,
		label: 'Output truncated',
		message: 'This update produced more than 50 MB of output. Only the first 50 MB is stored.'
	};
}

function terminalInlineBadgesFor(item: UpdateHistoryResponse) {
	const badges: Array<{ id: string; tone: 'warning' | 'info'; label: string }> = [];
	if (activeStreamId === item.id && item.interactive) {
		badges.push({
			id: 'interactive',
			tone: stdinAttention ? 'warning' : 'info',
			label: stdinAttention ? 'Interactive terminal' : 'Interactive terminal'
		});
	}
	return badges;
}

function terminalDetailsFor(item: UpdateHistoryResponse) {
	return [
		item.actor_type ? { id: 'actor', label: 'Actor', value: `${item.actor_type} (${item.actor_id})` } : null,
		item.pre_update_protection_summary
			? { id: 'pre-update', label: 'Protection summary', value: item.pre_update_protection_summary }
			: null,
		item.recovery_hint ? { id: 'recovery', label: 'Recovery hint', value: item.recovery_hint } : null
	].filter((detail): detail is { id: string; label: string; value: string } => detail !== null);
}

function terminalEmptyStateFor(item: UpdateHistoryResponse) {
	if (isWaitingStatus(item.status)) {
		return {
			label: item.status === 'queued' ? 'Queued' : 'Pending',
			message:
				item.status === 'queued'
					? 'Waiting for another update on this host to finish.'
					: 'Waiting for the agent to start the update.'
		};
	}
	if (!isLiveStatus(item.status) && !(item.output ?? '').trim()) {
		return { label: 'No output', message: 'No output recorded.' };
	}
	return undefined;
}
```

Then wire the terminal invocation to use the new props:

```svelte
<TerminalOutput
	bind:this={terminalRef}
	open={true}
	title={`${expandedItem.software_item_name} on ${expandedItem.host_name}`}
	statusLabel={terminalStatusLabelFor(expandedItem)}
	statusTone={terminalStatusToneFor(expandedItem)}
	metadata={terminalMetadataFor(expandedItem)}
	criticalBanner={terminalCriticalBannerFor(expandedItem)}
	inlineBadges={terminalInlineBadgesFor(expandedItem)}
	details={terminalDetailsFor(expandedItem)}
	emptyState={terminalEmptyStateFor(expandedItem)}
	showTerminal={isLiveStatus(expandedItem.status) || Boolean(expandedItem.output?.trim())}
	output={expandedItem.output ?? ''}
	actions={terminalActionsFor(expandedItem)}
	onInput={isLiveStatus(expandedItem.status)
		? (data) => (activeStreamId === expandedItem.id ? activeWsHandle?.sendInput(data) : undefined)
		: undefined}
	onclose={() => toggleExpand(expandedItem.id)}
/>
```

- [ ] **Step 4: Run the history-route tests again**

Run:

```bash
npm --prefix frontend test -- src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts
```

Expected: PASS with actor/recovery details hidden until `Details` is opened,
truncation rendered in the single banner slot, and waiting/no-output states
using the empty-state shell.

- [ ] **Step 5: Commit the history-route integration**

```bash
git add frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts frontend/src/routes/history/history-trigger-status.test.ts
git commit -m "feat(frontend): map history terminals to shared shell contract"
```

## Task 3: Update Software Live-Terminal Mapping

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Test: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`

- [ ] **Step 1: Write the failing software-route test**

Add a test that locks the software detail page into the new terminal contract instead of the route-local `Input Required` warning treatment:

```ts
it('uses shared inline badges instead of warning status overrides for the live terminal', async () => {
	const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
	vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));
	vi.mocked(api.triggerSoftwareUpdate).mockResolvedValue({
		update_history_id: 'uh-live',
		status: 'pending'
	});

	render(SoftwareDetailPage);
	await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());
	await fireEvent.click(screen.getByRole('button', { name: 'Update' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));

	const shell = await screen.findByRole('dialog', { name: 'Demo App on host-one' });
	expect(shell).toHaveAttribute('data-ui', 'terminal-shell');
	expect(document.querySelector('[data-ui="terminal-inline-badges"]')).toBeInTheDocument();
	expect(screen.queryByText('Input Required')).not.toBeInTheDocument();
	expect(document.querySelector('[data-ui="terminal-shell"] [data-ui="callout"]')).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the software-route test and verify it fails**

Run:

```bash
npm --prefix frontend test -- 'src/routes/software/[id]/software-detail-update-trigger.test.ts'
```

Expected: FAIL because the software route still overrides
`statusLabel`/`statusTone` to `Input Required` instead of passing compact
inline badges into `TerminalOutput`.

- [ ] **Step 3: Implement software-route mapping helpers**

Keep the live modal’s connection status in the existing status badge and move interactivity into `inlineBadges`.

```ts
function liveInlineBadges() {
	if (!liveModal) return [];
	return [
		{
			id: 'interactive',
			tone: liveStdinAttention ? 'warning' : 'info',
			label: 'Interactive terminal'
		}
	];
}
```

Then update the modal wiring:

```svelte
<TerminalOutput
	bind:this={liveTerminalRef}
	open={true}
	title={`${item?.name ?? 'Software item'} on ${liveModal.hostName}`}
	statusLabel={liveModalStatusLabel()}
	statusTone={liveModalStatusTone()}
	metadata={liveMetadata(liveModal.hostName)}
	inlineBadges={liveInlineBadges()}
	actions={liveTerminalActions()}
	onclose={closeLiveModal}
	onInput={liveWsState === 'connected' || liveWsState === 'connecting'
		? (data) => liveWsHandle?.sendInput(data)
		: undefined}
/>
```

- [ ] **Step 4: Run the software-route test again**

Run:

```bash
npm --prefix frontend test -- 'src/routes/software/[id]/software-detail-update-trigger.test.ts'
```

Expected: PASS with no route-local `Input Required` status override and no in-shell `Callout` usage.

- [ ] **Step 5: Commit the software-route integration**

```bash
git add 'frontend/src/routes/software/[id]/+page.svelte' 'frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts'
git commit -m "feat(frontend): align software live terminal with shared shell"
```

## Task 4: Run the Affected Frontend Verification Suite

**Files:**

- Modify: `frontend/src/lib/components/TerminalOutput.svelte` (only if verification exposes a bug)
- Modify: `frontend/src/routes/history/+page.svelte` (only if verification exposes a bug)
- Modify: `frontend/src/routes/software/[id]/+page.svelte` (only if verification exposes a bug)
- Test: `frontend/src/lib/components/TerminalOutput.test.ts`
- Test: `frontend/src/routes/history/history.test.ts`
- Test: `frontend/src/routes/history/history-trigger-status.test.ts`
- Test: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`

- [ ] **Step 1: Run the focused frontend test suite**

Run:

```bash
npm --prefix frontend test -- \
	src/lib/components/TerminalOutput.test.ts \
	src/routes/history/history.test.ts \
	src/routes/history/history-trigger-status.test.ts \
	'src/routes/software/[id]/software-detail-update-trigger.test.ts'
```

Expected: PASS with coverage for banner precedence, collapsed details, empty-state gating, and route integration.

- [ ] **Step 2: Run frontend type-check**

Run:

```bash
npm --prefix frontend run check
```

Expected: PASS with no new Svelte or TypeScript errors.

- [ ] **Step 3: Run frontend lint**

Run:

```bash
npm --prefix frontend run lint
```

Expected: PASS with no new ESLint violations.

- [ ] **Step 4: Commit verification-driven polish only if Steps 1-3 required additional code edits; otherwise skip this step**

```bash
git add frontend/src/lib/components/TerminalOutput.svelte frontend/src/routes/history/+page.svelte 'frontend/src/routes/software/[id]/+page.svelte'
git commit -m "fix(frontend): polish terminal redesign verification issues"
```

## Self-Review

- **Spec coverage:** Task 1 covers the shared terminal contract,
  single-banner slot, no-`Callout` rule, collapsed details, and empty-state
  gating. Task 2 covers history-route mapping and route-level precedence
  behavior. Task 3 covers the software route so the redesign applies
  everywhere the shared shell is used. Task 4 covers the verification commands
  required to prove the refactor is stable.
- **Placeholder scan:** No `TODO`, `TBD`, “similar to,” or
  “write tests for the above” placeholders remain. Every task contains
  concrete files, code, and commands.
- **Type consistency:** The prop names are consistent across tasks:
  `criticalBanner`, `inlineBadges`, `details`, and `emptyState`. Route tasks
  call the same prop names introduced in Task 1.

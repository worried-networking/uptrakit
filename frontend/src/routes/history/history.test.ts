import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { Permission, type UpdateHistoryResponse } from '$lib/types';
import { page } from '$app/state';

const interactiveMocks = vi.hoisted(() => ({
	disconnect: vi.fn(),
	sendSignal: vi.fn(),
	sendInput: vi.fn()
}));

vi.mock('$lib/api', () => ({
	listUpdateHistory: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	getSoftwareItems: vi.fn(),
	getUpdateHistoryEntry: vi.fn(),
	getSoftwareItem: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => interactiveMocks)
}));

vi.mock('$lib/sse', () => ({
	connectEventStream: vi.fn(() => () => {})
}));

import HistoryPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000102',
	email: 'history@example.com',
	first_name: 'History',
	last_name: 'User',
	has_pending_email_change: false,
	permissions: [Permission.ViewSoftware, Permission.TriggerUpdates]
};

const queuedItem = {
	id: 'hist-queued',
	host_id: 'host-1',
	host_name: 'prod-01',
	software_item_id: 'software-1',
	software_item_name: 'nginx',
	from_version: '1.24.0',
	to_version: '1.25.0',
	status: 'queued',
	started_at: '2026-02-01T10:00:00Z',
	completed_at: null,
	output: '',
	output_truncated: true,
	interactive: false,
	actor_type: 'user',
	actor_id: 'actor-1',
	actor_name: 'Alice Smith',
	created_at: '2026-02-01T10:00:00Z'
} satisfies UpdateHistoryResponse;

const completedItem = {
	id: 'hist-completed',
	host_id: 'host-5',
	host_name: 'prod-05',
	software_item_id: 'software-5',
	software_item_name: 'grafana',
	from_version: '11.0.0',
	to_version: '11.1.0',
	status: 'completed',
	started_at: '2026-02-01T08:00:00Z',
	completed_at: '2026-02-01T08:10:00Z',
	output: 'Completed.',
	output_truncated: false,
	interactive: false,
	actor_type: 'user',
	actor_id: 'actor-5',
	actor_name: 'Bob Jones',
	created_at: '2026-02-01T08:00:00Z'
} satisfies UpdateHistoryResponse;

const failedItem = {
	id: 'hist-failed',
	host_id: 'host-2',
	host_name: 'prod-02',
	software_item_id: 'software-2',
	software_item_name: 'redis',
	from_version: '7.0.0',
	to_version: '7.2.0',
	status: 'failed',
	started_at: '2026-01-31T09:30:00Z',
	completed_at: '2026-01-31T09:45:00Z',
	output: 'Error output',
	output_truncated: false,
	interactive: false,
	actor_type: 'user',
	actor_id: 'actor-2',
	actor_name: 'Carol Lee',
	created_at: '2026-01-31T09:30:00Z'
} satisfies UpdateHistoryResponse;

const inProgressItem = {
	id: 'hist-in-progress',
	host_id: 'host-3',
	host_name: 'prod-03',
	software_item_id: 'software-3',
	software_item_name: 'postgresql',
	from_version: '16.1',
	to_version: '16.2',
	status: 'in_progress',
	started_at: '2026-01-30T08:00:00Z',
	completed_at: null,
	output: '',
	output_truncated: false,
	interactive: true,
	actor_type: 'user',
	actor_id: 'actor-3',
	actor_name: 'Dave Kim',
	created_at: '2026-01-30T08:00:00Z'
} satisfies UpdateHistoryResponse;

const pendingItem = {
	id: 'hist-pending',
	host_id: 'host-4',
	host_name: 'prod-04',
	software_item_id: 'software-4',
	software_item_name: 'docker',
	from_version: '27.0.0',
	to_version: '27.1.0',
	status: 'pending',
	started_at: '2026-01-30T12:00:00Z',
	completed_at: null,
	output: '',
	output_truncated: false,
	interactive: false,
	actor_type: 'user',
	actor_id: 'actor-4',
	actor_name: 'Eve Park',
	created_at: '2026-01-30T12:00:00Z'
} satisfies UpdateHistoryResponse;

describe('History Route', () => {
	beforeAll(() => {
		class ResizeObserverMock {
			observe = vi.fn();
			disconnect = vi.fn();
		}
		vi.stubGlobal('ResizeObserver', ResizeObserverMock);
		vi.stubGlobal(
			'matchMedia',
			vi.fn(() => ({
				matches: false,
				media: '',
				onchange: null,
				addEventListener: vi.fn(),
				removeEventListener: vi.fn(),
				addListener: vi.fn(),
				removeListener: vi.fn(),
				dispatchEvent: vi.fn(() => false)
			}))
		);
	});

	beforeEach(() => {
		page.url.pathname = '/history';
		page.url.search = '';
		vi.useFakeTimers();
		vi.setSystemTime(new Date('2026-02-01T12:00:00Z'));
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [queuedItem, completedItem, failedItem, inProgressItem, pendingItem],
			total: 5,
			page: 1,
			per_page: 25,
			total_pages: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
		vi.useRealTimers();
	});

	afterAll(() => {
		vi.unstubAllGlobals();
	});

	it('renders chronological feed entries with required glyphs and date grouping', async () => {
		render(HistoryPage);

		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="history-feed-list"]')).toBeInTheDocument();
		const nginxEntryTitle = screen.getByText('nginx on prod-01');
		expect(nginxEntryTitle).toBeInTheDocument();
		const nginxEntry = nginxEntryTitle.closest('article');
		expect(nginxEntry).not.toBeNull();
		expect(nginxEntry).toHaveTextContent(/1\.24\.0\s*→\s*1\.25\.0/);
		expect(screen.getAllByRole('button', { name: /view logs/i }).length).toBeGreaterThan(0);
		expect(screen.getByText('Today')).toBeInTheDocument();
		expect(screen.getByText('Yesterday')).toBeInTheDocument();
		const glyphTexts = [...document.querySelectorAll('[data-ui="history-status-glyph"]')]
			.map((glyph) => glyph.textContent?.trim())
			.filter((value): value is string => Boolean(value));
		expect(glyphTexts).toEqual(expect.arrayContaining(['✓', '✕', '↑', '·']));
	});

	it('opens waiting-state output in the shared terminal modal shell', async () => {
		render(HistoryPage);

		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());
		const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
		const viewLogButton = within(nginxEntry).getByRole('button', { name: /view logs/i });
		expect(viewLogButton).not.toBeNull();
		await fireEvent.click(viewLogButton);

		const waitingMessage = await screen.findByText(/waiting for another update/i);
		const shell = document.querySelector('[data-ui="terminal-shell"]');
		expect(shell).toBeInTheDocument();
		expect(waitingMessage.closest('[data-ui="terminal-shell"]')).toBe(shell);
		expect(screen.getByText('Output truncated')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-critical-banner"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-empty-state"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-output"]')).not.toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-shell"] [data-ui="callout"]')).not.toBeInTheDocument();

		expect(screen.queryByText('user (actor-1)')).not.toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: /details/i }));
		expect(screen.getByText('Actor')).toBeInTheDocument();
		expect(screen.getByText('user (actor-1)')).toBeInTheDocument();
	});

	it('shows in-modal Ctrl+C for live entries and forwards SIGINT to the interactive session', async () => {
		render(HistoryPage);

		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());
		const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
		const viewLogButton = within(pgEntry).getByRole('button', { name: /attach terminal/i });
		expect(viewLogButton).not.toBeNull();
		await fireEvent.click(viewLogButton);
		vi.runOnlyPendingTimers();

		const sigintButton = await screen.findByRole('button', { name: 'Ctrl+C' });
		expect(sigintButton.closest('[data-ui="terminal-shell"]')).toBeInTheDocument();
		await fireEvent.click(sigintButton);
		expect(interactiveMocks.sendSignal).toHaveBeenCalledWith(2);
	});

	describe('filter chips', () => {
		it('renders inactive filter chip as ghost sm with no active class', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

			// 'Completed' chip should be inactive — statusFilter defaults to 'all'
			const completedChip = screen.getByRole('button', { name: 'Completed' });
			expect(completedChip).toBeInTheDocument();
			// ghost variant: has border border-[var(--border-default)]
			expect(completedChip.className).toContain('border-[var(--border-default)]');
			// no active override
			expect(completedChip.className).not.toContain('text-[var(--accent)]');
			expect(completedChip.className).not.toContain('bg-[var(--bg-hover)]');
		});

		it('renders active filter chip with solid accent variant', async () => {
			// Pre-set URL to status=completed so the chip renders active on mount
			page.url.search = '?status=completed';
			render(HistoryPage);
			await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

			const completedChip = screen.getByRole('button', { name: 'Completed' });
			expect(completedChip.className).toContain('bg-[var(--accent)]');
			expect(completedChip.className).toContain('text-[var(--text-inverted)]');
		});

		it('renders All chip as active by default', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

			const allChip = screen.getByRole('button', { name: 'All' });
			expect(allChip.className).toContain('bg-[var(--accent)]');
			expect(allChip.className).toContain('text-[var(--text-inverted)]');
		});
	});

	describe('per-row action buttons', () => {
		it('renders View logs for non-interactive idle row', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

			// completedItem: interactive=false, status=completed, not expanded
			const viewButtons = screen.getAllByRole('button', { name: /view logs/i });
			expect(viewButtons.length).toBeGreaterThan(0);
		});

		it('renders Attach terminal for interactive in_progress row when collapsed', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

			// inProgressItem: interactive=true, status=in_progress
			const attachButton = screen.getByRole('button', { name: /attach terminal/i });
			expect(attachButton).toBeInTheDocument();
		});
	});

	it('renders the summary strip only on page 1 with the all filter', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const summaryStrip = document.querySelector('[data-ui="history-summary-strip"]') as HTMLElement;
		expect(within(summaryStrip).getByText('Running')).toBeInTheDocument();
		expect(within(summaryStrip).getByText('Waiting')).toBeInTheDocument();
		expect(within(summaryStrip).getByText('Failed')).toBeInTheDocument();
		expect(within(summaryStrip).getByText('Completed')).toBeInTheDocument();
	});

	it('hides the summary strip for non-all filters and later pages', async () => {
		page.url.search = '?status=completed&page=2';
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [completedItem],
			total: 5,
			page: 2,
			per_page: 25,
			total_pages: 2
		});

		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		expect(document.querySelector('[data-ui="history-summary-strip"]')).toBeNull();
	});

	it('does not render the summary strip while the page-1 all-results load is pending', async () => {
		vi.mocked(api.listUpdateHistory).mockImplementation(
			() => new Promise(() => undefined) as ReturnType<typeof api.listUpdateHistory>
		);

		render(HistoryPage);
		expect(screen.getByText('Loading update history…')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="history-summary-strip"]')).toBeNull();
	});

	it('renders actor display names in collapsed row metadata', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
		expect(nginxEntry).toHaveTextContent('Triggered by user Alice Smith');
	});

	it('does not render the Input Required badge in the feed', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		expect(screen.queryByText(/input required/i)).not.toBeInTheDocument();
	});

	it('falls back to trigger source unknown when actor type is missing', async () => {
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [{ ...queuedItem, actor_type: '', actor_name: null }],
			total: 1,
			page: 1,
			per_page: 25,
			total_pages: 1
		});

		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		expect(screen.getByText('Trigger source unknown')).toBeInTheDocument();
	});

	it('falls back to type-only label when actor name is absent', async () => {
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [{ ...queuedItem, actor_type: 'user', actor_name: null }],
			total: 1,
			page: 1,
			per_page: 25,
			total_pages: 1
		});

		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		expect(screen.getByText('Triggered by user')).toBeInTheDocument();
	});

	it('renders summary bucket counts that match fixture data', async () => {
		// Default mock has 5 items:
		// queuedItem (queued)         -> Waiting
		// completedItem (completed)   -> Completed
		// failedItem (failed)         -> Failed
		// inProgressItem (in_progress)-> Running
		// pendingItem (pending)       -> Waiting
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const summaryStrip = document.querySelector('[data-ui="history-summary-strip"]') as HTMLElement;
		expect(summaryStrip).not.toBeNull();

		const bucketBy = (label: string) => within(summaryStrip).getByText(label).closest('div') as HTMLElement;

		expect(within(bucketBy('Running')).getByText('1')).toBeInTheDocument();
		expect(within(bucketBy('Waiting')).getByText('2')).toBeInTheDocument();
		expect(within(bucketBy('Failed')).getByText('1')).toBeInTheDocument();
		expect(within(bucketBy('Completed')).getByText('1')).toBeInTheDocument();
	});

	it('keeps stable visible row action labels after opening the modal', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
		const attachBtn = screen.getByRole('button', { name: 'Attach terminal' });
		await fireEvent.click(attachBtn);
		vi.runOnlyPendingTimers();

		expect(pgEntry).toHaveTextContent('Attach terminal');
		expect(within(pgEntry).queryByRole('button', { name: /close terminal/i })).not.toBeInTheDocument();
	});

	it('does not render aria-expanded on row actions', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const action = screen.getByRole('button', { name: 'Attach terminal' });
		expect(action).not.toHaveAttribute('aria-expanded');
	});

	it('does not close the modal when clicking the action for the already-open row', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const attachBtn = screen.getByRole('button', { name: 'Attach terminal' });
		await fireEvent.click(attachBtn);
		vi.runOnlyPendingTimers();
		expect(document.querySelector('[data-ui="terminal-shell"]')).toBeInTheDocument();

		await fireEvent.click(attachBtn);
		expect(document.querySelector('[data-ui="terminal-shell"]')).toBeInTheDocument();
	});

	it('retargets the existing modal when clicking a different row action', async () => {
		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

		const grafanaButton = screen
			.getByText('grafana on prod-05')
			.closest('article')!
			.querySelector('button') as HTMLElement;
		await fireEvent.click(grafanaButton);
		expect(await screen.findByRole('dialog', { name: 'grafana on prod-05' })).toBeInTheDocument();

		const secondButton = screen
			.getByText('nginx on prod-01')
			.closest('article')!
			.querySelector('button') as HTMLElement;
		await fireEvent.click(secondButton);

		expect(await screen.findByRole('dialog', { name: 'nginx on prod-01' })).toBeInTheDocument();
		expect(screen.queryByRole('dialog', { name: 'grafana on prod-05' })).not.toBeInTheDocument();
	});
});

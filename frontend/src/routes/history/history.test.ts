import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
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
	permissions: [Permission.ViewSoftware, Permission.TriggerUpdates]
};

const queuedItem: UpdateHistoryResponse = {
	id: 'hist-queued',
	host_name: 'prod-01',
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
	actor_id: 'actor-1'
} as unknown as UpdateHistoryResponse;

const completedItem: UpdateHistoryResponse = {
	id: 'hist-completed',
	host_name: 'prod-05',
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
	actor_id: 'actor-5'
} as unknown as UpdateHistoryResponse;

const failedItem: UpdateHistoryResponse = {
	id: 'hist-failed',
	host_name: 'prod-02',
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
	actor_id: 'actor-2'
} as unknown as UpdateHistoryResponse;

const inProgressItem: UpdateHistoryResponse = {
	id: 'hist-in-progress',
	host_name: 'prod-03',
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
	actor_id: 'actor-3'
} as unknown as UpdateHistoryResponse;

const pendingItem: UpdateHistoryResponse = {
	id: 'hist-pending',
	host_name: 'prod-04',
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
	actor_id: 'actor-4'
} as unknown as UpdateHistoryResponse;

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
		expect(screen.getAllByText('▶ view log').length).toBeGreaterThan(0);
		expect(screen.getByText('Today')).toBeInTheDocument();
		expect(screen.getByText('Yesterday')).toBeInTheDocument();
		const glyphTexts = [...document.querySelectorAll('[data-ui="history-status-glyph"]')]
			.map((glyph) => glyph.textContent?.trim())
			.filter((value): value is string => Boolean(value));
		expect(glyphTexts).toEqual(expect.arrayContaining(['✓', '✕', '↑', '·']));
	});

	it('opens waiting-state output in the shared terminal modal shell', async () => {
		render(HistoryPage);

		const viewLogButton = await screen.findByRole('button', {
			name: 'Expand output for nginx on prod-01'
		});
		await fireEvent.click(viewLogButton);

		const waitingMessage = await screen.findByText(/waiting for another update/i);
		const shell = document.querySelector('[data-ui="terminal-shell"]');
		expect(shell).toBeInTheDocument();
		expect(waitingMessage.closest('[data-ui="terminal-shell"]')).toBe(shell);
		expect(screen.getByText('Output truncated')).toBeInTheDocument();
		expect(screen.getByText('Actor')).toBeInTheDocument();
		expect(screen.getByText('user (actor-1)')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="history-feed-output"]')).not.toBeInTheDocument();
		expect(document.querySelectorAll('[data-ui="callout"]').length).toBeGreaterThanOrEqual(1);
	});

	it('shows in-modal Ctrl+C for live entries and forwards SIGINT to the interactive session', async () => {
		render(HistoryPage);

		const viewLogButton = await screen.findByRole('button', {
			name: 'Expand output for postgresql on prod-03'
		});
		await fireEvent.click(viewLogButton);
		vi.runOnlyPendingTimers();

		const sigintButton = await screen.findByRole('button', { name: 'Ctrl+C' });
		expect(sigintButton.closest('[data-ui="terminal-shell"]')).toBeInTheDocument();
		await fireEvent.click(sigintButton);
		expect(interactiveMocks.sendSignal).toHaveBeenCalledWith(2);
	});
});

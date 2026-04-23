import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listNotificationRules: vi.fn(),
	listNotificationChannels: vi.fn(),
	createNotificationRule: vi.fn(),
	updateNotificationRule: vi.fn(),
	deleteNotificationRule: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import * as api from '$lib/api';
import NotificationRulesSettings from './NotificationRulesSettings.svelte';

const defaultProps = {
	onSuccess: vi.fn(),
	onError: vi.fn()
};

function stubApis() {
	vi.mocked(api.listNotificationRules).mockResolvedValue({
		items: [],
		total: 0,
		page: 1,
		per_page: 20,
		total_pages: 1
	});
	vi.mocked(api.listNotificationChannels).mockResolvedValue({
		items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
		total: 1,
		page: 1,
		per_page: 20,
		total_pages: 1
	});
}

afterEach(() => vi.clearAllMocks());

describe('NotificationRulesSettings — button variants', () => {
	it('Add Rule button has no raw preset-filled-primary-500 class', async () => {
		stubApis();
		const { container } = render(NotificationRulesSettings, defaultProps);
		await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Add Rule button has primary variant class (accent gradient)', async () => {
		stubApis();
		render(NotificationRulesSettings, defaultProps);
		await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
		const btn = screen.getByRole('button', { name: 'Add Rule' });
		expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('Add Rule button has sm size class (h-[19px])', async () => {
		stubApis();
		render(NotificationRulesSettings, defaultProps);
		await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
		const btn = screen.getByRole('button', { name: 'Add Rule' });
		expect(btn.className).toContain('h-[19px]');
	});

	it('per-row Edit button has secondary variant class and sm size', async () => {
		vi.mocked(api.listNotificationRules).mockResolvedValue({
			items: [
				{
					id: 'r1',
					channel_id: 'ch1',
					event_type: 'update_available',
					host_id: null,
					software_item_id: null,
					plugin_type: null,
					enabled: true,
					created_at: '2026-01-01T00:00:00Z'
				}
			],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		render(NotificationRulesSettings, defaultProps);
		const btn = await screen.findByRole('button', { name: 'Edit' });
		expect(btn.className).toContain('bg-[var(--bg-raised)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('per-row Delete button has danger variant class and sm size', async () => {
		vi.mocked(api.listNotificationRules).mockResolvedValue({
			items: [
				{
					id: 'r1',
					channel_id: 'ch1',
					event_type: 'update_available',
					host_id: null,
					software_item_id: null,
					plugin_type: null,
					enabled: true,
					created_at: '2026-01-01T00:00:00Z'
				}
			],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		render(NotificationRulesSettings, defaultProps);
		const btn = await screen.findByRole('button', { name: 'Delete' });
		expect(btn.className).toContain('bg-[var(--color-error-bg)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('pagination Previous/Next buttons have secondary variant and sm size', async () => {
		vi.mocked(api.listNotificationRules).mockResolvedValue({
			items: [
				{
					id: 'r1',
					channel_id: 'ch1',
					event_type: 'update_available',
					host_id: null,
					software_item_id: null,
					plugin_type: null,
					enabled: true,
					created_at: '2026-01-01T00:00:00Z'
				}
			],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 3
		});
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		render(NotificationRulesSettings, defaultProps);
		const prev = await screen.findByRole('button', { name: 'Previous' });
		const next = screen.getByRole('button', { name: 'Next' });
		expect(prev.className).toContain('bg-[var(--bg-raised)]');
		expect(prev.className).toContain('h-[19px]');
		expect(next.className).toContain('bg-[var(--bg-raised)]');
		expect(next.className).toContain('h-[19px]');
	});

	it('modal submit carries aria-busy=true while save is in flight', async () => {
		stubApis();
		let resolve!: () => void;
		vi.mocked(api.createNotificationRule).mockReturnValue(
			new Promise((r) => {
				resolve = () => r({} as never);
			})
		);
		render(NotificationRulesSettings, defaultProps);
		await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());

		await fireEvent.click(screen.getByRole('button', { name: 'Add Rule' }));
		const submitBtn = await screen.findByRole('button', { name: 'Create' });
		await fireEvent.click(submitBtn);

		await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));
		expect(submitBtn).toHaveTextContent('Create');

		// Resolve the in-flight save; saveRule calls loadData() after success so stub it again
		vi.mocked(api.listNotificationRules).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		resolve();
		// After resolve the modal closes, so the button is removed from the DOM.
		// We only need to confirm aria-busy was set to true (already asserted above).
	});

	it('modal submit text is Update when editing a rule', async () => {
		vi.mocked(api.listNotificationRules).mockResolvedValue({
			items: [
				{
					id: 'r1',
					channel_id: 'ch1',
					event_type: 'update_available',
					host_id: null,
					software_item_id: null,
					plugin_type: null,
					enabled: true,
					created_at: '2026-01-01T00:00:00Z'
				}
			],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		render(NotificationRulesSettings, defaultProps);
		const editBtn = await screen.findByRole('button', { name: 'Edit' });
		await fireEvent.click(editBtn);
		await screen.findByRole('button', { name: 'Update' });
	});
});

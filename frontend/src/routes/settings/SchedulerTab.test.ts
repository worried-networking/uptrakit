import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listSchedulerTasks: vi.fn().mockResolvedValue([
		{
			id: 'task-1',
			name: 'Test Task',
			label: 'Test Task',
			cron_expression: '0 * * * *',
			is_enabled: true,
			is_running: false,
			last_run_at: null,
			next_run_at: null,
			interval_seconds: 3600,
			jitter_seconds: 0,
			enabled: true,
			task_type: 'custom',
			last_error: null
		}
	]),
	updateSchedulerTask: vi.fn(),
	triggerSchedulerTask: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		has_pending_email_change: false,
		permissions: ['manage_scheduler']
	}))
}));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));

import * as api from '$lib/api';
import { Permission } from '$lib/types';
import * as auth from '$lib/auth.svelte';
import SchedulerTab from './SchedulerTab.svelte';

afterEach(() => vi.clearAllMocks());

function makeUser() {
	return {
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		has_pending_email_change: false,
		permissions: [Permission.ManageScheduler]
	} as ReturnType<typeof auth.getUser>;
}

describe('SchedulerTab button variants', () => {
	it('has no raw preset-filled-primary-500 buttons', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		const { container } = render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('has no raw preset-tonal-surface buttons', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		const { container } = render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		expect(container.querySelector('button.preset-tonal-surface')).toBeNull();
	});

	it('Run button has ghost (bg-transparent) class', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		const runBtn = screen.getByRole('button', { name: 'Run' });
		expect(runBtn.className).toContain('bg-transparent');
	});

	it('Save button carries aria-busy=true while saving', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		let resolve!: (v: unknown) => void;
		vi.mocked(api.updateSchedulerTask).mockReturnValue(
			new Promise((r) => {
				resolve = r as unknown as (v: unknown) => void;
			})
		);
		render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
		const saveBtn = await screen.findByRole('button', { name: 'Save' });
		await fireEvent.click(saveBtn);
		await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
		resolve({
			id: 'task-1',
			name: 'Test Task',
			label: 'Test Task',
			cron_expression: '0 * * * *',
			is_enabled: true,
			is_running: false,
			last_run_at: null,
			next_run_at: null,
			interval_seconds: 3600,
			jitter_seconds: 0,
			enabled: true,
			task_type: 'custom',
			last_error: null
		});
		// After save completes, modal closes - button is removed from DOM; just wait for aria-busy to clear
		// or the button to be gone (either is acceptable proof that saving completed)
		await waitFor(() => {
			const stillBusy = saveBtn.getAttribute('aria-busy') === 'true';
			const stillInDom = document.body.contains(saveBtn);
			expect(stillBusy && stillInDom).toBe(false);
		});
	});

	it('Cancel button has secondary class', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
		const cancelBtn = await screen.findByRole('button', { name: 'Cancel' });
		expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
	});

	it('Retry button (on load error) has primary gradient class', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		vi.mocked(api.listSchedulerTasks).mockRejectedValueOnce(new Error('load failed'));
		render(SchedulerTab);
		const retryBtn = await screen.findByRole('button', { name: 'Retry' });
		expect(retryBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('Save button text is static "Save" during loading — no text swap', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());
		let resolve!: (v: unknown) => void;
		vi.mocked(api.updateSchedulerTask).mockReturnValue(
			new Promise((r) => {
				resolve = r as unknown as (v: unknown) => void;
			})
		);
		render(SchedulerTab);
		await waitFor(() => screen.getByText('Test Task'));
		await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
		const saveBtn = await screen.findByRole('button', { name: 'Save' });
		await fireEvent.click(saveBtn);
		await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
		expect(saveBtn).toHaveTextContent('Save');
		resolve({});
	});
});

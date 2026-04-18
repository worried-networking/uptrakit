import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import ToastNotifications from './ToastNotifications.svelte';
import type { SystemAlert } from '$lib/types';

const notificationState = vi.hoisted(() => ({
	successMessage: null as string | null,
	errorMessage: null as string | null
}));

vi.mock('$lib/notifications.svelte', () => ({
	getSuccessMessage: vi.fn(() => notificationState.successMessage),
	getErrorMessage: vi.fn(() => notificationState.errorMessage),
	clearError: vi.fn(() => {
		notificationState.errorMessage = null;
	})
}));

afterEach(() => {
	cleanup();
	notificationState.successMessage = null;
	notificationState.errorMessage = null;
	vi.clearAllMocks();
	vi.useRealTimers();
});

describe('ToastNotifications', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	it('uses a single live-region owner per toast without wrapper duplication', () => {
		notificationState.successMessage = 'Update completed';
		notificationState.errorMessage = 'Update failed';

		const alerts: SystemAlert[] = [
			{
				id: 'alert-warning',
				severity: 'warning',
				title: 'Certificate warning',
				message: 'Certificate expires soon'
			}
		];

		const { container } = render(ToastNotifications, { alerts, onDismiss: vi.fn() });
		const toastCards = Array.from(container.querySelectorAll<HTMLElement>('[data-ui="toast-notification"]'));

		expect(toastCards).toHaveLength(3);

		for (const toastCard of toastCards) {
			expect(toastCard).toHaveAttribute('role', 'group');
			expect(toastCard).not.toHaveAttribute('aria-live');
			expect(toastCard).not.toHaveAttribute('aria-atomic');

			const liveRegionOwners = toastCard.querySelectorAll('[role="status"], [role="alert"]');
			expect(liveRegionOwners).toHaveLength(1);
			expect(liveRegionOwners[0]).toHaveAttribute('data-ui', 'callout');
		}
	});

	it('maps error and critical system alerts to danger tone with alert urgency', () => {
		const alerts: SystemAlert[] = [
			{
				id: 'alert-error',
				severity: 'error',
				title: 'Error alert',
				message: 'Failed operation'
			},
			{
				id: 'alert-critical',
				severity: 'critical',
				title: 'Critical alert',
				message: 'Immediate action required'
			}
		];

		const { container } = render(ToastNotifications, { alerts, onDismiss: vi.fn() });
		const toastCards = Array.from(container.querySelectorAll<HTMLElement>('[data-ui="toast-notification"]'));

		expect(toastCards).toHaveLength(2);

		for (const title of ['Error alert', 'Critical alert']) {
			const toastCard = screen.getByText(title).closest('[data-ui="toast-notification"]');
			expect(toastCard).toBeInTheDocument();
			expect(toastCard).toHaveAttribute('role', 'group');
			expect(toastCard).not.toHaveAttribute('aria-live');

			const utils = within(toastCard as HTMLElement);
			const callout = utils.getByText(title).closest('[data-ui="callout"]');
			expect(callout).toHaveAttribute('data-tone', 'danger');
			expect(callout).toHaveAttribute('role', 'alert');

			const statusBadge = toastCard?.querySelector('[data-ui="status-badge"]');
			expect(statusBadge).toHaveAttribute('data-tone', 'danger');
		}
	});

	it('auto-dismisses success toasts after 4 seconds and exposes a countdown progress bar', async () => {
		notificationState.successMessage = 'Saved changes';

		const { container } = render(ToastNotifications, { alerts: [], onDismiss: vi.fn() });

		const toast = screen.getByText('Saved changes').closest('[data-ui="toast-notification"]');
		expect(toast).toBeInTheDocument();
		expect(toast?.querySelector('[data-ui="toast-progress"]')).toBeInTheDocument();

		await vi.advanceTimersByTimeAsync(3999);
		expect(screen.getByText('Saved changes')).toBeInTheDocument();

		await vi.advanceTimersByTimeAsync(1);
		await Promise.resolve();
		expect(container.querySelector('[data-ui="toast-notification"]')).toBeNull();
	});

	it('pauses success toast auto-dismiss on hover and resumes on mouse leave', async () => {
		notificationState.successMessage = 'Queued update';
		render(ToastNotifications, { alerts: [], onDismiss: vi.fn() });

		const toast = screen.getByText('Queued update').closest('[data-ui="toast-notification"]') as HTMLElement;
		expect(toast).toBeInTheDocument();

		await fireEvent.mouseEnter(toast);
		await vi.advanceTimersByTimeAsync(6000);
		expect(screen.getByText('Queued update')).toBeInTheDocument();

		await fireEvent.mouseLeave(toast);
		await vi.advanceTimersByTimeAsync(3999);
		expect(screen.getByText('Queued update')).toBeInTheDocument();
		await vi.advanceTimersByTimeAsync(1);
		await Promise.resolve();
		expect(screen.queryByText('Queued update')).not.toBeInTheDocument();
	});

	it('auto-dismisses warning system alerts after 8 seconds', () => {
		const onDismiss = vi.fn();
		const alerts: SystemAlert[] = [
			{
				id: 'alert-warning',
				severity: 'warning',
				title: 'Warning alert',
				message: 'Something needs attention'
			}
		];
		render(ToastNotifications, { alerts, onDismiss });

		vi.advanceTimersByTime(7999);
		expect(onDismiss).not.toHaveBeenCalled();

		vi.advanceTimersByTime(1);
		expect(onDismiss).toHaveBeenCalledWith('alert-warning');
	});
});

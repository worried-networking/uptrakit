import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/svelte';
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
});

describe('ToastNotifications', () => {
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
			expect(toastCard).not.toHaveAttribute('role');
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
			expect(toastCard).not.toHaveAttribute('role');
			expect(toastCard).not.toHaveAttribute('aria-live');

			const utils = within(toastCard as HTMLElement);
			const callout = utils.getByText(title).closest('[data-ui="callout"]');
			expect(callout).toHaveAttribute('data-tone', 'danger');
			expect(callout).toHaveAttribute('role', 'alert');

			const statusBadge = toastCard?.querySelector('[data-ui="status-badge"]');
			expect(statusBadge).toHaveAttribute('data-tone', 'danger');
		}
	});
});

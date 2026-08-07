import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';

vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));
vi.mock('$lib/utils', () => ({
	formatDate: (d: string) => d
}));
vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	getConfigState: vi.fn(),
	clearCoordinatorDegraded: vi.fn()
}));

import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import InstanceConfigTab from './InstanceConfigTab.svelte';
import type { ConfigStateResponse } from '$lib/api';
import { Actions } from '$lib/api';

const idleState: ConfigStateResponse = {
	coordinator_state: 'idle',
	degraded: null,
	file: {
		path: '/etc/uptrakit/controller.toml',
		digest: 'abc123',
		loaded_at: '2026-05-12T10:00:00Z',
		pending_digest: null,
		pending_detected_at: null
	},
	last_reload: null,
	sections: {
		db: { url: '<redacted>' },
		network: { https_addr: '[::]:8443', pki_addr: '[::]:8444' },
		tls: { trust_domain: 'uptrakit.local', cert_path: '/etc/uptrakit/tls.pem' }
	},
	recent_events: []
};

const degradedState: ConfigStateResponse = {
	...idleState,
	coordinator_state: 'degraded',
	degraded: {
		since: '2026-05-12T09:00:00Z',
		failed_subsystems: ['db'],
		reason: 'DB pool failed to apply new URL'
	}
};

describe('InstanceConfigTab', () => {
	beforeEach(() => {
		vi.mocked(api.getConfigState).mockResolvedValue({ data: idleState } as unknown as Awaited<
			ReturnType<typeof api.getConfigState>
		>);
	});
	afterEach(() => vi.clearAllMocks());

	it('shows file path after load', async () => {
		render(InstanceConfigTab);
		await screen.findByText('/etc/uptrakit/controller.toml');
	});

	it('shows degraded banner when state is degraded', async () => {
		vi.mocked(api.getConfigState).mockResolvedValue({ data: degradedState } as unknown as Awaited<
			ReturnType<typeof api.getConfigState>
		>);
		render(InstanceConfigTab);
		await screen.findByText('Coordinator degraded');
	});

	it('clear degraded button hidden without manage permission', async () => {
		vi.mocked(api.getConfigState).mockResolvedValue({ data: degradedState } as unknown as Awaited<
			ReturnType<typeof api.getConfigState>
		>);
		vi.mocked(auth.getUser).mockReturnValue({
			id: '1',
			email: 'a@b.com',
			first_name: 'A',
			last_name: 'B',
			actions: [Actions.SYSTEM_CONFIG_STATE_READ],
			authority: 'ok',
			has_pending_email_change: false
		});
		render(InstanceConfigTab);
		await screen.findByText('Coordinator degraded');
		expect(screen.queryByRole('button', { name: /clear degraded/i })).toBeNull();
	});

	it('clear degraded button shown with manage permission', async () => {
		vi.mocked(api.getConfigState).mockResolvedValue({ data: degradedState } as unknown as Awaited<
			ReturnType<typeof api.getConfigState>
		>);
		vi.mocked(auth.getUser).mockReturnValue({
			id: '1',
			email: 'a@b.com',
			first_name: 'A',
			last_name: 'B',
			actions: [Actions.SYSTEM_CONFIG_STATE_READ, Actions.SYSTEM_CONFIG_STATE_MANAGE],
			authority: 'ok',
			has_pending_email_change: false
		});
		render(InstanceConfigTab);
		await screen.findByRole('button', { name: /clear degraded/i });
	});

	it('sections show redacted secrets', async () => {
		render(InstanceConfigTab);
		await screen.findByText(/<redacted>/);
	});

	it('shows no "Reload Now" button', async () => {
		render(InstanceConfigTab);
		await screen.findByText('/etc/uptrakit/controller.toml');
		expect(screen.queryByRole('button', { name: /reload now/i })).toBeNull();
	});
});

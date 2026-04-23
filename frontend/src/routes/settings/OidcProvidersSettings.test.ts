import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	getOidcProviders: vi.fn(),
	createOidcProvider: vi.fn(),
	updateOidcProvider: vi.fn(),
	deleteOidcProvider: vi.fn(),
	activateOidcProvider: vi.fn(),
	deactivateOidcProvider: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import OidcProvidersSettings from './OidcProvidersSettings.svelte';

const defaultProps = {
	providers: [],
	multiTenancyEnabled: false,
	onSuccess: vi.fn(),
	onError: vi.fn()
};

function makeProvider(id: string, name: string, isActive: boolean) {
	return {
		id,
		name,
		slug: id,
		logo_url: null,
		issuer_url: 'https://issuer.example.com',
		client_id: 'client_id',
		scopes: 'openid',
		auto_create_users: true,
		allow_private_network_issuers: false,
		role_mapping: {},
		role_claim_path: null,
		is_active: isActive
	};
}

afterEach(() => vi.clearAllMocks());

describe('OidcProvidersSettings — button variants', () => {
	it('Add Provider button has no raw preset-filled-primary-500 class', () => {
		const { container } = render(OidcProvidersSettings, defaultProps);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Add Provider button has primary variant class (accent gradient)', () => {
		render(OidcProvidersSettings, defaultProps);
		const btn = screen.getByRole('button', { name: 'Add Provider' });
		expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('per-row Edit button has secondary variant and sm size', () => {
		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', false)]
		});
		const btn = screen.getByRole('button', { name: 'Edit' });
		expect(btn.className).toContain('bg-[var(--bg-raised)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('per-row Activate button has secondary variant and sm size', () => {
		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', false)]
		});
		const btn = screen.getByRole('button', { name: 'Activate' });
		expect(btn.className).toContain('bg-[var(--bg-raised)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('per-row Deactivate button has secondary variant and sm size', () => {
		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', true)]
		});
		const btn = screen.getByRole('button', { name: 'Deactivate' });
		expect(btn.className).toContain('bg-[var(--bg-raised)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('per-row Delete button has danger variant and sm size', () => {
		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', false)]
		});
		const btn = screen.getByRole('button', { name: 'Delete' });
		expect(btn.className).toContain('bg-[var(--color-error-bg)]');
		expect(btn.className).toContain('h-[19px]');
	});

	it('only the toggled row has aria-busy=true — other rows remain unaffected', async () => {
		let resolveToggle!: () => void;
		vi.mocked(api.deactivateOidcProvider).mockReturnValue(
			new Promise((r) => {
				resolveToggle = () => r(makeProvider('p1', 'Provider One', false));
			})
		);
		vi.mocked(api.getOidcProviders).mockResolvedValue([
			makeProvider('p1', 'Provider One', true),
			makeProvider('p2', 'Provider Two', true)
		]);

		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', true), makeProvider('p2', 'Provider Two', true)]
		});

		const deactivateBtns = screen.getAllByRole('button', { name: 'Deactivate' });
		expect(deactivateBtns).toHaveLength(2);

		await fireEvent.click(deactivateBtns[0]);
		await waitFor(() => expect(deactivateBtns[0]).toHaveAttribute('aria-busy', 'true'));
		expect(deactivateBtns[1]).not.toHaveAttribute('aria-busy');

		resolveToggle();
		await waitFor(() => expect(deactivateBtns[0]).not.toHaveAttribute('aria-busy'));
	});

	it('modal submit shows Create text and carries aria-busy during save', async () => {
		let resolve!: () => void;
		vi.mocked(api.createOidcProvider).mockReturnValue(
			new Promise((r) => {
				resolve = () => r(makeProvider('p-new', 'New Provider', false));
			})
		);

		render(OidcProvidersSettings, defaultProps);
		await fireEvent.click(screen.getByRole('button', { name: 'Add Provider' }));

		const submitBtn = await screen.findByRole('button', { name: 'Create' });
		expect(submitBtn).toBeDefined();

		await fireEvent.click(submitBtn);
		await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));

		// After resolve, saveOidcProvider calls closeOidcModal() which unmounts the button.
		// We only need to confirm aria-busy was set to true (already asserted above).
		resolve();
	});

	it('modal submit shows Update text when editing', async () => {
		render(OidcProvidersSettings, {
			...defaultProps,
			providers: [makeProvider('p1', 'Provider One', false)]
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
		await screen.findByRole('button', { name: 'Update' });
	});

	it('modal Cancel button has secondary variant', async () => {
		render(OidcProvidersSettings, defaultProps);
		await fireEvent.click(screen.getByRole('button', { name: 'Add Provider' }));
		const cancelBtn = await screen.findByRole('button', { name: 'Cancel' });
		expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
	});
});

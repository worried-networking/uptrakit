import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	listClients: vi.fn().mockResolvedValue({ data: { items: [] } }),
	revokeClient: vi.fn(),
	trustClient: vi.fn()
}));
vi.mock('$lib/api/oauth', () => ({
	getOAuthSettings: vi.fn(),
	updateOAuthSettings: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		has_pending_email_change: false,
		actions: ['settings.auth:manage', 'system.settings:manage'],
		authority: 'ok'
	}))
}));

import * as oauthApi from '$lib/api/oauth';
import type { OAuthSettingsResponse } from '$lib/api/oauth';
import McpAccessTab from './McpAccessTab.svelte';

function defaultOAuthSettings(overrides: Partial<OAuthSettingsResponse> = {}): OAuthSettingsResponse {
	return {
		mcp_enabled: false,
		dcr_enabled: false,
		cimd_enabled: false,
		restart_required: false,
		canonical_host: null,
		...overrides
	};
}

describe('McpAccessTab OAuth Settings card', () => {
	afterEach(() => vi.clearAllMocks());

	function oauthSection() {
		return screen.getByRole('heading', { name: 'OAuth Settings' }).closest('[data-ui="section-card"]') as HTMLElement;
	}

	it('does not render a canonical_host field', async () => {
		vi.mocked(oauthApi.getOAuthSettings).mockResolvedValue(defaultOAuthSettings());
		const { container } = render(McpAccessTab);
		await screen.findByText('OAuth Settings');
		await waitFor(() => expect(oauthApi.getOAuthSettings).toHaveBeenCalledOnce());
		expect(container.querySelector('#canonical_host')).toBeNull();
		expect(container.querySelector('#canonical-host')).toBeNull();
	});

	it('shows a pointer to the Global Settings Canonical Host card', async () => {
		vi.mocked(oauthApi.getOAuthSettings).mockResolvedValue(defaultOAuthSettings());
		render(McpAccessTab);
		await screen.findByText(/Canonical host is configured under Global Settings/);
	});

	it('save payload omits canonical_host', async () => {
		vi.mocked(oauthApi.getOAuthSettings).mockResolvedValue(defaultOAuthSettings());
		vi.mocked(oauthApi.updateOAuthSettings).mockResolvedValue(defaultOAuthSettings({ mcp_enabled: true }));
		render(McpAccessTab);
		await screen.findByText('OAuth Settings');
		const section = oauthSection();

		const mcpCheckbox = document.getElementById('mcp_enabled') as HTMLInputElement;
		await fireEvent.click(mcpCheckbox);

		const saveBtn = await waitFor(() => {
			const btn = Array.from(section.querySelectorAll('button')).find((b) => b.textContent?.trim() === 'Save');
			if (!btn || btn.hasAttribute('disabled')) throw new Error('save not ready');
			return btn;
		});
		await fireEvent.click(saveBtn);

		await waitFor(() => expect(oauthApi.updateOAuthSettings).toHaveBeenCalledOnce());
		const payload = vi.mocked(oauthApi.updateOAuthSettings).mock.calls[0][0];
		expect(payload).not.toHaveProperty('canonical_host');
	});

	it('shows the warning callout when mcp_enabled is true and no canonical host is loaded', async () => {
		vi.mocked(oauthApi.getOAuthSettings).mockResolvedValue(
			defaultOAuthSettings({ mcp_enabled: true, canonical_host: null })
		);
		render(McpAccessTab);
		await screen.findByText(/oauth.canonical_host must be set before enabling MCP OAuth/);
	});
});

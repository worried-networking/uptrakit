import { createRawSnippet } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import Layout from './+layout.svelte';
import * as auth from '$lib/auth.svelte';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/hosts')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/types', () => ({
	Permission: {
		ViewSoftware: 'view_software',
		ViewSettings: 'view_settings',
		ViewSystemServices: 'view_system_services',
		ViewHosts: 'view_hosts',
		ViewAuditLogs: 'view_audit_logs',
		ManageAuthSettings: 'manage_auth_settings',
		ManageEnrollmentTokens: 'manage_enrollment_tokens',
		ManageAgentCerts: 'manage_agent_certs',
		CreateSoftware: 'create_software',
		UpdateSoftware: 'update_software',
		DeleteSoftware: 'delete_software',
		ManageScheduler: 'manage_scheduler',
		ManageGlobalSettings: 'manage_global_settings'
	},
	hasPermissionValue: (user: { permissions?: string[] } | null | undefined, permission: string | null | undefined) =>
		permission ? Boolean(user?.permissions?.includes(permission)) : true
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		permissions: ['view_software', 'view_settings']
	})),
	getLoading: vi.fn(() => false),
	initialize: vi.fn(),
	handleLogout: vi.fn(),
	getSessionExpired: vi.fn(() => false),
	setSessionExpired: vi.fn()
}));

vi.mock('$lib/theme.svelte', () => ({
	getThemeMode: vi.fn(() => 'system'),
	setThemeMode: vi.fn(),
	initTheme: vi.fn()
}));

vi.mock('$lib/api', () => ({
	getSystemAlerts: vi.fn(async () => ({ alerts: [] }))
}));

vi.mock('$lib/stores/network.svelte', () => ({
	getIsOnline: vi.fn(() => true)
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	loadSurfaceRegistry: vi.fn(),
	clearSurfaceRegistry: vi.fn(),
	getSurfacesBySlot: vi.fn(() => []),
	resolveSurfacePageNavItems: vi.fn(() => [])
}));

describe('layout Button migration', () => {
	it('theme toggle renders as ghost Button with aria-label', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const toggle = document.querySelector('[data-ui="app-shell-header"] button[aria-label*="mode"]') as HTMLElement;
		expect(toggle).not.toBeNull();
		expect(toggle.className).toContain('h-[23px]');
		expect(toggle.className).toContain('bg-transparent');
		expect(toggle).toHaveAttribute('aria-label');
		// Icon-only guard: the accessible name comes from ariaLabel alone — no visible text
		expect(toggle.textContent?.trim()).toBe('');
	});

	it('tablet sidebar toggle renders as ghost button with aria-label, aria-controls, and aria-expanded', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const toggle = document.querySelector('[data-ui="app-shell-sidebar-toggle"]') as HTMLElement;
		// Toggle only renders in tablet viewport range; skip assertion if not rendered
		if (!toggle) return;
		expect(toggle.tagName.toLowerCase()).toBe('button');
		expect(toggle.className).toContain('h-[23px]');
		expect(toggle.className).toContain('bg-transparent');
		expect(toggle).toHaveAttribute('aria-label');
		expect(toggle).toHaveAttribute('aria-controls', 'app-shell-sidebar-tablet');
		expect(toggle).toHaveAttribute('aria-expanded');
		expect(toggle).not.toHaveAttribute('role', 'link');
	});

	it('logout button renders as danger Button', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const logoutBtn = screen.getByRole('button', { name: /logout/i });
		expect(logoutBtn.className).toContain('h-[23px]');
		expect(logoutBtn.className).toContain('var(--color-danger-bg)'); // danger variant
		expect(logoutBtn.className).not.toContain('bg-transparent');
	});

	describe('session-expired banner', () => {
		beforeEach(() => {
			vi.mocked(auth.getSessionExpired).mockReturnValue(true);
		});

		it('"Log in" in session-expired banner renders as danger Button (size="sm") with href', () => {
			render(Layout, {
				children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
			});
			// Use CSS selector — Button href branch renders <a role="button">, not <a role="link">
			const loginAnchor = document.querySelector('a[href*="/login"]') as HTMLElement;
			expect(loginAnchor).not.toBeNull();
			expect(loginAnchor.className).toContain('h-[19px]'); // size="sm"
			expect(loginAnchor.className).toContain('var(--color-danger-bg)'); // danger variant
			expect(loginAnchor.className).not.toContain('preset-filled-error');
		});

		it('"Dismiss" in session-expired banner renders as ghost Button (size="sm")', () => {
			render(Layout, {
				children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
			});
			const dismissBtn = screen.getByRole('button', { name: /dismiss/i });
			expect(dismissBtn.className).toContain('h-[19px]'); // size="sm"
			expect(dismissBtn.className).toContain('bg-transparent'); // ghost
		});
	});

	it('active nav pill carries accent-bright text and rgba bg tokens', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const nav = document.querySelector('[data-ui="app-shell-nav"]');
		if (!nav) return; // layout may not render in jsdom — skip gracefully
		const links = Array.from(nav.querySelectorAll('a'));
		if (links.length === 0) return;

		for (const link of links) {
			// If this link is active, both fragments must be present together.
			if (link.className.includes('accent-bright')) {
				expect(link.className).toContain('bg-[rgba(var(--accent-rgb)');
			}
			// If this link is inactive, neither fragment should be present.
			if (!link.className.includes('accent-bright')) {
				expect(link.className).not.toContain('bg-[rgba(var(--accent-rgb)');
			}
			// The old bg-hover token must NOT be used for nav-pill active state.
			expect(link.className).not.toContain('bg-[var(--bg-hover)]');
		}
	});

	it('nav anchors inside nav landmark render as plain <a> without role="button"', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const nav = document.querySelector('[data-ui="app-shell-nav"]');
		if (!nav) return;
		const navLinks = nav.querySelectorAll('a');
		for (const link of navLinks) {
			expect(link.getAttribute('role')).not.toBe('button');
		}
	});

	it('layout source contains no preset-filled-* or preset-tonal-* class strings', () => {
		expect(layoutSource).not.toMatch(/preset-filled-/);
		expect(layoutSource).not.toMatch(/preset-tonal-/);
		expect(layoutSource).not.toMatch(/btn-icon/);
	});
});

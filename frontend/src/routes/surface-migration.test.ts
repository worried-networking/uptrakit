import { createRawSnippet } from 'svelte';
import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import surfacePageSource from './surfaces/[id]/+page.svelte?raw';
import settingsPageSource from './settings/+page.svelte?raw';
import globalSettingsTabSource from './settings/GlobalSettingsTab.svelte?raw';
import softwarePageSource from './software/+page.svelte?raw';
import softwareDetailPageSource from './software/[id]/+page.svelte?raw';
import Layout from './+layout.svelte';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/software')
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
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: true })),
	getSurfacesBySlot: vi.fn(() => []),
	resolveSurfacePageNavItems: vi.fn(() => [
		{
			id: 'surface.software.z',
			href: '/surfaces/surface.software.z',
			label: 'Software',
			priority: 500
		},
		{
			id: 'surface.software.a',
			href: '/surfaces/surface.software.a',
			label: 'Software',
			priority: 500
		}
	])
}));

const migratedRouteSources = [
	layoutSource,
	surfacePageSource,
	settingsPageSource,
	globalSettingsTabSource,
	softwarePageSource,
	softwareDetailPageSource
];

describe('shared-surface route migration', () => {
	it('uses shared-surface runtime modules in migrated routes', () => {
		for (const content of migratedRouteSources) {
			expect(content).toContain('$lib/surfaces');
		}
	});

	it('keeps migrated routes on the canonical shared-surface page path', () => {
		for (const content of migratedRouteSources) {
			expect(content).not.toContain('/extensions/');
			expect(content).toContain('/surfaces/');
		}
	});

	it('keeps deterministic sidebar ordering for built-in and surface pages with equal labels', () => {
		render(Layout, {
			children: createRawSnippet(() => ({
				render: () => '<p>Shell content</p>'
			}))
		});

		const navLinks = within(screen.getByRole('navigation')).getAllByRole('link');
		const navLabels = navLinks.map((node) => node.textContent?.trim() ?? '');
		const navHrefs = navLinks.map((node) => node.getAttribute('href'));

		expect(navLabels).toContain('Software');
		expect(navLabels).toContain('Settings');
		expect(navHrefs).toEqual(
			expect.arrayContaining(['/software', '/surfaces/surface.software.a', '/surfaces/surface.software.z'])
		);
		expect(navHrefs.indexOf('/software')).toBeLessThan(navHrefs.indexOf('/surfaces/surface.software.a'));
		expect(navHrefs.indexOf('/surfaces/surface.software.a')).toBeLessThan(
			navHrefs.indexOf('/surfaces/surface.software.z')
		);
	});
});

import { createRawSnippet } from 'svelte';
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import Layout from './+layout.svelte';

// Regression guard for the "two scrollbars on surface pages" bug.
//
// Symptom: surface pages (e.g. /surfaces/proxmox.hosts) rendered two vertical
// scrollbars — one on <main>, and a second on documentElement scrolling the
// entire layout (header + sidebar). /hosts was unaffected.
//
// Root cause: Chrome computes documentElement.scrollHeight from the natural
// layout extent of in-flow descendants. When a surface table renders many
// rows with row_action <td> cells, the layout extent of the table escapes
// <main>'s overflow-auto clip, so document scrollHeight grows past viewport
// even though <main>'s height is correctly bounded. Every surface table that
// renders row actions hits this; /hosts hand-rolls its own action cell and
// happens to avoid the trigger.
//
// Fix: `contain: layout` on <main>. Layout containment isolates <main>'s
// internal layout extent from documentElement scroll calculation, so
// documentElement.scrollHeight cannot exceed viewport regardless of which
// descendants render or what CSS features they use.
//
// If `contain-layout` is removed from <main>, the bug returns silently —
// these assertions catch that.

vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/hosts') } }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
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
		has_pending_email_change: false,
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
vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	getSystemAlerts: vi.fn(async () => ({ data: { alerts: [] } }))
}));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));
vi.mock('$lib/stores/software-updates.svelte', () => ({
	getUpdatableSoftwareCount: vi.fn(() => null),
	fetchUpdatableSoftwareCount: vi.fn(async () => {})
}));
vi.mock('$lib/surfaces/registry.svelte', () => ({
	loadSurfaceRegistry: vi.fn(),
	clearSurfaceRegistry: vi.fn(),
	getSurfacesBySlot: vi.fn(() => []),
	resolveSurfacePageNavItems: vi.fn(() => [])
}));
vi.mock('$lib/stores/events.svelte', () => ({ subscribeToEvent: vi.fn(() => vi.fn()) }));

describe('layout scroll containment', () => {
	afterEach(() => cleanup());

	it('layout source declares contain-layout on <main>', () => {
		// Tightly anchored to the actual <main> tag so renames or class
		// reorderings still pass, but accidental removal of contain-layout
		// fails loudly.
		expect(layoutSource).toMatch(/<main\b[^>]*\bclass="[^"]*\bcontain-layout\b[^"]*"/);
	});

	it('rendered <main> element has contain-layout class', () => {
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const main = document.getElementById('main-content');
		expect(main).not.toBeNull();
		expect(main?.classList.contains('contain-layout')).toBe(true);
	});

	it('rendered <main> still owns its overflow-auto scroll context', () => {
		// The fix relies on contain:layout PLUS overflow-auto on the same
		// element. If overflow-auto migrates elsewhere the containment
		// guarantee no longer holds — surface this drift early.
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const main = document.getElementById('main-content');
		expect(main?.classList.contains('overflow-auto')).toBe(true);
	});
});

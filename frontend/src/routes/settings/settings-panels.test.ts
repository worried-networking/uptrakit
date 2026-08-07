import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { Actions, PluginCapability } from '$lib/api';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	listEnrollmentTokens: vi.fn(),
	createEnrollmentToken: vi.fn(),
	revokeEnrollmentToken: vi.fn(),
	listSystemEnrollmentTokens: vi.fn(),
	createSystemEnrollmentToken: vi.fn(),
	revokeSystemEnrollmentToken: vi.fn(),
	listLog: vi.fn(),
	listChannels: vi.fn(),
	listPluginConfigs: vi.fn(),
	createPluginConfig: vi.fn(),
	updatePluginConfig: vi.fn(),
	deletePluginConfig: vi.fn(),
	discoverPluginConfig: vi.fn(),
	listTenantDiscoveryAllowlist: vi.fn(),
	addTenantDiscoveryAllowlistEntry: vi.fn(),
	removeTenantDiscoveryAllowlistEntry: vi.fn(),
	listPluginTypes: vi.fn(),
	batchPluginConfigs: vi.fn(),
	listPluginTypeSettings: vi.fn(),
	upsertPluginTypeSettings: vi.fn(),
	deletePluginTypeSettings: vi.fn(),
	testPluginConfig: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';
import SystemServicesSettings from './SystemServicesSettings.svelte';
import NotificationLogView from './NotificationLogView.svelte';
import PluginConfigsTab from './PluginConfigsTab.svelte';

function deferred<T>() {
	let resolve!: (value: T | PromiseLike<T>) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe('settings panels design-language alignment', () => {
	beforeEach(() => {
		vi.mocked(api.listEnrollmentTokens).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>);
		vi.mocked(api.listSystemEnrollmentTokens).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>);
		vi.mocked(api.listLog).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 50, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listLog>>);
		vi.mocked(api.listChannels).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 50, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listChannels>>);
		vi.mocked(api.listPluginTypes).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypes>
		>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);
		vi.mocked(auth.getUser).mockReturnValue({
			id: 'user-1',
			email: 'settings@example.com',
			first_name: 'Settings',
			last_name: 'User',
			has_pending_email_change: false,
			actions: [
				Actions.SOFTWARE_READ,
				Actions.COMMANDS_MANAGE,
				Actions.CHECKS_TRIGGER,
				Actions.SOFTWARE_UPDATE,
				Actions.SETTINGS_READ,
				Actions.SYSTEM_SETTINGS_MANAGE,
				Actions.PLUGIN_CONFIGS_TRIGGER
			],
			authority: 'ok'
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('uses shared form rows and inline required validation in enrollment token create dialog', async () => {
		vi.mocked(api.listEnrollmentTokens).mockResolvedValue({
			data: {
				items: [
					{
						id: 'tok-1',
						name: 'Deploy Token',
						allowed_capabilities: [],
						max_uses: 10,
						current_uses: 1,
						expires_at: null,
						revoked_at: null,
						created_at: '2026-04-01T10:00:00Z',
						created_by_user_id: null
					}
				],
				total: 2,
				page: 1,
				per_page: 1,
				total_pages: 2
			}
		} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>);

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await screen.findByText('Deploy Token');
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		const title = await screen.findByText('Create Enrollment Token');
		expect(title.closest('[data-ui="modal-shell"]')).toBeInTheDocument();
		const formRows = title.closest('[data-ui="modal-shell"]')?.querySelectorAll('[data-ui="form-field-row"]');
		expect(formRows?.length).toBe(4);

		const nameInput = screen.getByLabelText('Name');
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Name is required.')).toBeInTheDocument();
		expect(nameInput).toHaveAttribute('aria-invalid', 'true');
		expect(vi.mocked(api.createEnrollmentToken)).not.toHaveBeenCalled();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
	});

	it('uses known-shape loading treatment while enrollment tokens are pending', async () => {
		const tokensDeferred = deferred<Awaited<ReturnType<typeof api.listEnrollmentTokens>>>();
		vi.mocked(api.listEnrollmentTokens).mockReturnValue(tokensDeferred.promise as never);

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		expect(document.querySelector('[data-ui="known-shape-table-loading"]')).toBeInTheDocument();
		expect(screen.getByText('Capabilities')).toBeInTheDocument();
		expect(
			document.querySelector('[data-ui="known-shape-table-loading"] [data-ui="loading-skeleton-cell"]')
		).toBeInTheDocument();

		tokensDeferred.resolve({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>);

		await waitFor(() => {
			expect(document.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		});
	});

	it('uses shared success callout for created enrollment token reveal', async () => {
		vi.mocked(api.createEnrollmentToken).mockResolvedValue({
			data: {
				id: 'created-token-1',
				token: 'secret-enrollment-token',
				name: 'Deploy Token',
				allowed_capabilities: null,
				max_uses: null,
				current_uses: 0,
				expires_at: null,
				created_at: '2026-04-01T11:00:00Z',
				created_by_user_id: null
			}
		} as unknown as Awaited<ReturnType<typeof api.createEnrollmentToken>>);

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Deploy Token' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Token created — copy it now, it will not be shown again')).toBeInTheDocument();
		const callout = screen.getByText('secret-enrollment-token').closest('[data-ui="callout"]');
		expect(callout).toBeInTheDocument();
		expect(callout).toHaveAttribute('data-tone', 'success');
		expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
	});

	it('shows error state when enrollment token page load fails (no Refresh button)', async () => {
		let pageTwoAttempts = 0;
		vi.mocked(api.listEnrollmentTokens).mockImplementation((async (opts: { query?: { page?: number } } = {}) => {
			const page = opts.query?.page ?? 1;
			if (page === 2) {
				pageTwoAttempts += 1;
				if (pageTwoAttempts === 1) {
					throw new Error('page two failed');
				}
				return {
					data: {
						items: [
							{
								id: 'tok-page-2',
								name: 'Page Two Token',
								allowed_capabilities: [],
								max_uses: null,
								current_uses: 0,
								expires_at: null,
								revoked_at: null,
								created_at: '2026-04-01T11:00:00Z',
								created_by_user_id: null
							}
						],
						total: 2,
						page: 2,
						per_page: 1,
						total_pages: 2
					}
				} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>;
			}
			return {
				data: {
					items: [
						{
							id: 'tok-page-1',
							name: 'Page One Token',
							allowed_capabilities: [],
							max_uses: null,
							current_uses: 0,
							expires_at: null,
							revoked_at: null,
							created_at: '2026-04-01T10:00:00Z',
							created_by_user_id: null
						}
					],
					total: 2,
					page: 1,
					per_page: 1,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>;
		}) as unknown as typeof api.listEnrollmentTokens);

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await screen.findByText('Page One Token');
		await fireEvent.click(screen.getByRole('button', { name: '2' }));
		expect(await screen.findByText('Unable to load data')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Refresh' })).not.toBeInTheDocument();
	});

	it('uses shared confirm dialog shell for enrollment token revoke flow', async () => {
		vi.mocked(api.listEnrollmentTokens).mockResolvedValue({
			data: {
				items: [
					{
						id: 'tok-active-1',
						name: 'Active Token',
						allowed_capabilities: null,
						max_uses: null,
						current_uses: 0,
						expires_at: null,
						revoked_at: null,
						created_at: '2026-04-01T10:00:00Z',
						created_by_user_id: null
					}
				],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listEnrollmentTokens>>);

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await fireEvent.click(await screen.findByRole('button', { name: 'Revoke' }));

		expect(await screen.findByText('Revoke Enrollment Token')).toBeInTheDocument();
		const dialog = screen.getByText('Revoke Enrollment Token').closest('[data-ui="modal-shell"]');
		expect(dialog).toBeInTheDocument();
		expect(within(dialog as HTMLElement).getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		expect(within(dialog as HTMLElement).getByRole('button', { name: 'Revoke' })).toBeInTheDocument();
	});

	it('uses shared form rows and inline required validation in system enrollment token create dialog', async () => {
		vi.mocked(api.listSystemEnrollmentTokens).mockResolvedValue({
			data: {
				items: [
					{
						id: 'sys-tok-1',
						name: 'Scheduler Token',
						max_uses: 10,
						current_uses: 2,
						expires_at: null,
						revoked_at: null,
						created_at: '2026-04-01T10:00:00Z',
						created_by_user_id: null
					}
				],
				total: 2,
				page: 1,
				per_page: 1,
				total_pages: 2
			}
		} as unknown as Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>);

		render(SystemServicesSettings, {
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await screen.findByText('Scheduler Token');
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		const title = await screen.findByText('Create System Enrollment Token');
		expect(title.closest('[data-ui="modal-shell"]')).toBeInTheDocument();
		const formRows = title.closest('[data-ui="modal-shell"]')?.querySelectorAll('[data-ui="form-field-row"]');
		expect(formRows?.length).toBe(3);

		const nameInput = screen.getByLabelText('Name');
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Name is required.')).toBeInTheDocument();
		expect(nameInput).toHaveAttribute('aria-invalid', 'true');
		expect(vi.mocked(api.createSystemEnrollmentToken)).not.toHaveBeenCalled();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
	});

	it('uses known-shape loading treatment while system enrollment tokens are pending', async () => {
		const tokensDeferred = deferred<Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>>();
		vi.mocked(api.listSystemEnrollmentTokens).mockReturnValue(tokensDeferred.promise as never);

		render(SystemServicesSettings, {
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		expect(document.querySelector('[data-ui="known-shape-table-loading"]')).toBeInTheDocument();
		expect(screen.getByText('Usage')).toBeInTheDocument();
		expect(
			document.querySelector('[data-ui="known-shape-table-loading"] [data-ui="loading-skeleton-cell"]')
		).toBeInTheDocument();

		tokensDeferred.resolve({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>);

		await waitFor(() => {
			expect(document.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		});
	});

	it('uses shared success callout for created system enrollment token reveal', async () => {
		vi.mocked(api.createSystemEnrollmentToken).mockResolvedValue({
			data: {
				id: 'created-system-token-1',
				token: 'secret-system-token',
				name: 'Scheduler Token',
				max_uses: null,
				current_uses: 0,
				expires_at: null,
				created_at: '2026-04-01T11:00:00Z',
				created_by_user_id: null
			}
		} as unknown as Awaited<ReturnType<typeof api.createSystemEnrollmentToken>>);

		render(SystemServicesSettings, {
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Scheduler Token' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Token created — copy it now, it will not be shown again')).toBeInTheDocument();
		const callout = screen.getByText('secret-system-token').closest('[data-ui="callout"]');
		expect(callout).toBeInTheDocument();
		expect(callout).toHaveAttribute('data-tone', 'success');
		expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
	});

	it('shows error state when system enrollment token page load fails (no Refresh button)', async () => {
		let pageTwoAttempts = 0;
		vi.mocked(api.listSystemEnrollmentTokens).mockImplementation((async (opts: { query?: { page?: number } } = {}) => {
			const page = opts.query?.page ?? 1;
			if (page === 2) {
				pageTwoAttempts += 1;
				if (pageTwoAttempts === 1) {
					throw new Error('page two failed');
				}
				return {
					data: {
						items: [
							{
								id: 'sys-tok-page-2',
								name: 'System Page Two Token',
								max_uses: null,
								current_uses: 0,
								expires_at: null,
								revoked_at: null,
								created_at: '2026-04-01T11:00:00Z',
								created_by_user_id: null
							}
						],
						total: 2,
						page: 2,
						per_page: 1,
						total_pages: 2
					}
				} as unknown as Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>;
			}
			return {
				data: {
					items: [
						{
							id: 'sys-tok-page-1',
							name: 'System Page One Token',
							max_uses: null,
							current_uses: 0,
							expires_at: null,
							revoked_at: null,
							created_at: '2026-04-01T10:00:00Z',
							created_by_user_id: null
						}
					],
					total: 2,
					page: 1,
					per_page: 1,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof api.listSystemEnrollmentTokens>>;
		}) as unknown as typeof api.listSystemEnrollmentTokens);

		render(SystemServicesSettings, {
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		await screen.findByText('System Page One Token');
		await fireEvent.click(screen.getByRole('button', { name: '2' }));
		expect(await screen.findByText('Unable to load data')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Refresh' })).not.toBeInTheDocument();
	});

	it('uses shared loading treatment for notification log while data is pending', async () => {
		const logDeferred = deferred<{
			data: {
				items: Array<Record<string, unknown>>;
				total: number;
				page: number;
				per_page: number;
				total_pages: number;
			};
		}>();
		vi.mocked(api.listLog).mockReturnValue(logDeferred.promise as never);
		vi.mocked(api.listChannels).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 50, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listChannels>>);

		render(NotificationLogView);

		expect(document.querySelector('[data-ui="known-shape-table-loading"]')).toBeInTheDocument();
		expect(screen.getByText('Event Type')).toBeInTheDocument();
		expect(screen.getByText('Delivered')).toBeInTheDocument();
		expect(
			document.querySelector('[data-ui="known-shape-table-loading"] [data-ui="loading-skeleton-cell"]')
		).toBeInTheDocument();

		logDeferred.resolve({
			data: { items: [], total: 0, page: 1, per_page: 50, total_pages: 1 }
		});

		await waitFor(() => {
			expect(document.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		});
	});

	it('uses shared empty and error treatments for notification log', async () => {
		vi.mocked(api.listLog).mockRejectedValue(new Error('boom'));
		vi.mocked(api.listChannels).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 50, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listChannels>>);

		render(NotificationLogView);

		expect(await screen.findByText('Unable to load data')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
	});

	it('uses shared table language and footer for notification log pagination', async () => {
		vi.mocked(api.listLog).mockResolvedValue({
			data: {
				items: [
					{
						id: 'log-1',
						event_type: 'update_available',
						channel_id: 'chan-1',
						rule_id: 'rule-1',
						status: 'delivered',
						created_at: '2026-04-01T11:00:00Z',
						delivered_at: '2026-04-01T11:01:00Z',
						error_message: null
					}
				],
				total: 2,
				page: 1,
				per_page: 1,
				total_pages: 2
			}
		} as unknown as Awaited<ReturnType<typeof api.listLog>>);
		vi.mocked(api.listChannels).mockResolvedValue({
			data: {
				items: [{ id: 'chan-1', name: 'Ops Email' }],
				total: 1,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listChannels>>);

		render(NotificationLogView);

		await screen.findByText('Update Available');
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('2 total')).toBeInTheDocument();
	});

	it('loads notification log only once per page change', async () => {
		const pageTwoDeferred = deferred<{
			data: {
				items: Array<Record<string, unknown>>;
				total: number;
				page: number;
				per_page: number;
				total_pages: number;
			};
		}>();

		vi.mocked(api.listLog).mockImplementation((opts) => {
			const page = (opts as { query?: { page?: number } } | undefined)?.query?.page ?? 1;
			if (page === 2) {
				return pageTwoDeferred.promise as never;
			}
			return Promise.resolve({
				data: {
					items: [
						{
							id: 'log-page-1',
							event_type: 'update_available',
							channel_id: 'chan-1',
							rule_id: 'rule-1',
							status: 'delivered',
							created_at: '2026-04-01T11:00:00Z',
							delivered_at: '2026-04-01T11:01:00Z',
							error_message: null
						}
					],
					total: 2,
					page: 1,
					per_page: 1,
					total_pages: 2
				}
			}) as never;
		});
		vi.mocked(api.listChannels).mockResolvedValue({
			data: {
				items: [{ id: 'chan-1', name: 'Ops Email' }],
				total: 1,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listChannels>>);

		render(NotificationLogView);

		await screen.findByText('Update Available');
		await fireEvent.click(screen.getByRole('button', { name: '2' }));

		await waitFor(() => {
			expect(vi.mocked(api.listLog)).toHaveBeenCalledTimes(2);
			expect(vi.mocked(api.listLog)).toHaveBeenLastCalledWith({ query: { page: 2 } });
		});

		pageTwoDeferred.resolve({
			data: {
				items: [
					{
						id: 'log-page-2',
						event_type: 'update_completed',
						channel_id: 'chan-1',
						rule_id: 'rule-1',
						status: 'delivered',
						created_at: '2026-04-02T11:00:00Z',
						delivered_at: '2026-04-02T11:01:00Z',
						error_message: null
					}
				],
				total: 2,
				page: 2,
				per_page: 1,
				total_pages: 2
			}
		});

		expect(await screen.findByText('Update Completed')).toBeInTheDocument();
	});

	it('renders shared table error treatment for failed enrollment token loads', async () => {
		vi.mocked(api.listEnrollmentTokens).mockRejectedValue(new Error('enrollment tokens failed'));

		render(EnrollmentTokenSettings, {
			summary: undefined,
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		expect(await screen.findByText('Unable to load data')).toBeInTheDocument();
		expect(screen.queryByText('No enrollment tokens configured.')).not.toBeInTheDocument();
	});

	it('renders shared table error treatment for failed system enrollment token loads', async () => {
		vi.mocked(api.listSystemEnrollmentTokens).mockRejectedValue(new Error('system enrollment tokens failed'));

		render(SystemServicesSettings, {
			onSuccess: vi.fn(),
			onError: vi.fn()
		});

		expect(await screen.findByText('Unable to load data')).toBeInTheDocument();
		expect(screen.queryByText('No system enrollment tokens configured.')).not.toBeInTheDocument();
	});

	it('keeps type defaults panel visible and uses its own shared empty state when no defaults are exposed', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [PluginCapability.DISCOVER_LOCAL_SOFTWARE],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: []
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [
					{
						id: 'cfg-1',
						name: 'Main GitHub',
						plugin_type: 'releases.github',
						config: {},
						enabled: true,
						capabilities: [PluginCapability.DISCOVER_LOCAL_SOFTWARE],
						created_at: '2026-04-01T11:00:00Z',
						updated_at: '2026-04-01T11:00:00Z'
					}
				],
				total: 2,
				page: 1,
				per_page: 1,
				total_pages: 2
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({
			data: [
				{
					id: 'allow-1',
					plugin_type: 'releases.github',
					created_at: '2026-04-01T11:00:00Z'
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listTenantDiscoveryAllowlist>>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await screen.findByText('Main GitHub');
		expect(screen.getByText('Type Defaults')).toBeInTheDocument();
		const typeDefaultsSection = screen.getByText('Type Defaults').closest('[data-ui="section-card"]');
		expect(typeDefaultsSection).toBeInTheDocument();
		expect(within(typeDefaultsSection as HTMLElement).getByText('No type defaults available.')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(document.querySelector('table.table')).not.toBeInTheDocument();
	});

	it('uses known-shape loading treatment while plugin settings tables are pending', async () => {
		const configsDeferred = deferred<Awaited<ReturnType<typeof api.listPluginConfigs>>>();
		const allowlistDeferred = deferred<Awaited<ReturnType<typeof api.listTenantDiscoveryAllowlist>>>();
		const typeSettingsDeferred = deferred<Awaited<ReturnType<typeof api.listPluginTypeSettings>>>();

		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [PluginCapability.DISCOVER_LOCAL_SOFTWARE],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: [
						{
							key: 'registry_url',
							label: 'Registry URL',
							field_type: 'text'
						}
					]
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockReturnValue(configsDeferred.promise as never);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockReturnValue(
			allowlistDeferred.promise as ReturnType<typeof api.listTenantDiscoveryAllowlist>
		);
		vi.mocked(api.listPluginTypeSettings).mockReturnValue(typeSettingsDeferred.promise as never);

		render(PluginConfigsTab);

		await waitFor(() => {
			expect(document.querySelectorAll('[data-ui="known-shape-table-loading"]').length).toBe(3);
		});
		expect(screen.getByText('Current Settings')).toBeInTheDocument();
		expect(
			document.querySelector('[data-ui="known-shape-table-loading"] [data-ui="loading-skeleton-cell"]')
		).toBeInTheDocument();

		configsDeferred.resolve({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		allowlistDeferred.resolve({ data: [] } as unknown as Awaited<ReturnType<typeof api.listTenantDiscoveryAllowlist>>);
		typeSettingsDeferred.resolve({ data: [] } as unknown as Awaited<ReturnType<typeof api.listPluginTypeSettings>>);

		await waitFor(() => {
			expect(document.querySelectorAll('[data-ui="known-shape-table-loading"]').length).toBe(0);
		});
	});

	it('clears plugin config batch selection when changing pages', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: []
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockImplementation((async (opts: { query?: { page?: number } } = {}) => {
			const page = opts.query?.page ?? 1;
			if (page === 2) {
				return {
					data: {
						items: [
							{
								id: 'cfg-2',
								name: 'Config Two',
								plugin_type: 'releases.github',
								config: {},
								enabled: true,
								capabilities: [],
								created_at: '2026-04-01T12:00:00Z',
								updated_at: '2026-04-01T12:00:00Z'
							}
						],
						total: 2,
						page: 2,
						per_page: 1,
						total_pages: 2
					}
				} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>;
			}
			return {
				data: {
					items: [
						{
							id: 'cfg-1',
							name: 'Config One',
							plugin_type: 'releases.github',
							config: {},
							enabled: true,
							capabilities: [],
							created_at: '2026-04-01T11:00:00Z',
							updated_at: '2026-04-01T11:00:00Z'
						}
					],
					total: 2,
					page: 1,
					per_page: 1,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>;
		}) as unknown as typeof api.listPluginConfigs);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await screen.findByText('Config One');
		await fireEvent.click(screen.getByLabelText('Select Config One'));
		expect(screen.getByText('1 selected')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: '2' }));
		await screen.findByText('Config Two');
		expect(screen.queryByText('1 selected')).not.toBeInTheDocument();
	});

	it('uses shared form-row rhythm and inline required validation in plugin config modal', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [PluginCapability.DISCOVER_LOCAL_SOFTWARE],
					sample_config: {},
					config_form_fields: [
						{
							key: 'api_token',
							label: 'API Token',
							field_type: 'text',
							required: true
						}
					],
					type_settings_form_fields: []
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await fireEvent.click(screen.getByRole('button', { name: 'Add Config' }));
		const title = await screen.findByText('Add Plugin Config');
		const modal = title.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(modal).toBeInTheDocument();
		expect(modal.querySelectorAll('[data-ui="form-field-row"]').length).toBeGreaterThanOrEqual(4);

		const pluginTypeSelect = screen.getByLabelText('Plugin Type');
		await fireEvent.change(pluginTypeSelect, { target: { value: 'releases.github' } });
		expect(await screen.findByLabelText('API Token')).toBeInTheDocument();

		await fireEvent.click(within(modal).getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Name is required.')).toBeInTheDocument();
		expect(screen.getByText('API Token is required.')).toBeInTheDocument();
		expect(vi.mocked(api.createPluginConfig)).not.toHaveBeenCalled();
		expect(screen.getByLabelText('Name')).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByLabelText('API Token')).toHaveAttribute('aria-invalid', 'true');
	});

	it('surfaces required schema validation while plugin config modal is in JSON mode', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [],
					sample_config: {},
					config_form_fields: [
						{
							key: 'api_token',
							label: 'API Token',
							field_type: 'text',
							required: true
						}
					],
					type_settings_form_fields: []
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await fireEvent.click(screen.getByRole('button', { name: 'Add Config' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'JSON Config' } });
		await fireEvent.change(screen.getByLabelText('Plugin Type'), { target: { value: 'releases.github' } });
		expect(await screen.findByLabelText('API Token')).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Advanced: Edit as JSON' }));
		await fireEvent.input(screen.getByLabelText('Config (JSON)'), { target: { value: '{}' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('API Token is required.')).toBeInTheDocument();
		expect(vi.mocked(api.createPluginConfig)).not.toHaveBeenCalled();
	});

	it('uses shared form-row rhythm and inline required validation in type defaults modal', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: false,
					capabilities: [],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: [
						{
							key: 'registry_url',
							label: 'Registry URL',
							field_type: 'text',
							required: true
						}
					]
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await waitFor(() => {
			expect(screen.getByRole('button', { name: 'Edit' })).toBeInTheDocument();
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
		const title = await screen.findByText('Edit Type Defaults — GitHub Releases');
		const modal = title.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(modal).toBeInTheDocument();
		expect(modal.querySelectorAll('[data-ui="form-field-row"]').length).toBeGreaterThanOrEqual(1);

		await fireEvent.click(within(modal).getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('Registry URL is required.')).toBeInTheDocument();
		expect(vi.mocked(api.upsertPluginTypeSettings)).not.toHaveBeenCalled();
		expect(screen.getByLabelText('Registry URL')).toHaveAttribute('aria-invalid', 'true');
	});

	it('uses shared form-row rhythm and inline required validation in discovery allowlist modal', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypes>
		>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);

		render(PluginConfigsTab);

		await fireEvent.click(screen.getByRole('button', { name: 'Add Plugin Type' }));
		const title = await screen.findByText('Add Discovery Plugin Type');
		const modal = title.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(modal).toBeInTheDocument();
		expect(modal.querySelectorAll('[data-ui="form-field-row"]').length).toBeGreaterThanOrEqual(1);

		await fireEvent.click(within(modal).getByRole('button', { name: 'Add' }));

		expect(await screen.findByText('Plugin type is required.')).toBeInTheDocument();
		expect(screen.getByLabelText('Plugin Type')).toHaveAttribute('aria-invalid', 'true');
		expect(vi.mocked(api.addTenantDiscoveryAllowlistEntry)).not.toHaveBeenCalled();
	});

	it('renders plugin config test results inside shared callout treatment', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: []
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockResolvedValue({
			data: {
				items: [],
				total: 0,
				page: 1,
				per_page: 50,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listPluginConfigs>>);
		vi.mocked(api.listTenantDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listTenantDiscoveryAllowlist>
		>);
		vi.mocked(api.listPluginTypeSettings).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypeSettings>
		>);
		vi.mocked(api.testPluginConfig).mockResolvedValue({
			data: {
				success: true,
				test_kind: 'version_check',
				detected_version: '1.2.3',
				duration_ms: 12
			}
		} as unknown as Awaited<ReturnType<typeof api.testPluginConfig>>);

		render(PluginConfigsTab);

		// Wait for plugin types to load so openCreateConfig sets configForm.plugin_type
		await screen.findByText('No plugin configs');

		await fireEvent.click(screen.getByRole('button', { name: 'Add Config' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'GitHub Config' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Test' }));

		expect(await screen.findByText('Test Passed')).toBeInTheDocument();
		expect(screen.getByText('Detected version:')).toBeInTheDocument();
		const callout = screen.getByText('Test Passed').closest('[data-ui="callout"]');
		expect(callout).toBeInTheDocument();
		expect(callout).toHaveAttribute('data-tone', 'success');
	});

	it('renders shared table error states for plugin config list panels when fetches fail', async () => {
		vi.mocked(api.listPluginTypes).mockResolvedValue({
			data: [
				{
					plugin_type: 'releases.github',
					display_name: 'GitHub Releases',
					supports_plugin_configs: true,
					capabilities: [PluginCapability.DISCOVER_LOCAL_SOFTWARE],
					sample_config: {},
					config_form_fields: [],
					type_settings_form_fields: [
						{
							key: 'registry_url',
							label: 'Registry URL',
							field_type: 'text'
						}
					]
				} as never
			]
		} as unknown as Awaited<ReturnType<typeof api.listPluginTypes>>);
		vi.mocked(api.listPluginConfigs).mockRejectedValue(new Error('configs failed'));
		vi.mocked(api.listTenantDiscoveryAllowlist).mockRejectedValue(new Error('allowlist failed'));
		vi.mocked(api.listPluginTypeSettings).mockRejectedValue(new Error('type settings failed'));

		render(PluginConfigsTab);

		const errorTitles = await screen.findAllByText('Unable to load data');
		expect(errorTitles.length).toBe(3);
		expect(screen.queryByText('No plugin configs')).not.toBeInTheDocument();
		expect(screen.queryByText('No restrictions — all discovery plugins are active.')).not.toBeInTheDocument();
		expect(screen.queryByText('No type defaults available.')).not.toBeInTheDocument();
	});
});

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { PaginatedResponse, ServiceResponse } from '$lib/types';
import { Permission } from '$lib/types';

// vi.mock calls are hoisted before imports by vitest — set them up first.
vi.mock('$lib/api', () => ({
	getServices: vi.fn(),
	approveService: vi.fn(),
	rejectService: vi.fn(),
	deleteService: vi.fn(),
	mergeService: vi.fn(),
	updateService: vi.fn(),
	batchServices: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAccessToken: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

import ServicesPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.ApproveServices,
		Permission.RejectServices,
		Permission.RemoveServices,
		Permission.UpdateServices
	]
};

function makePage(items: ServiceResponse[]): PaginatedResponse<ServiceResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

const approvedAgent: ServiceResponse = {
	id: 'svc-001',
	friendly_name: 'prod-agent',
	capabilities: ['software_discovery', 'update_hooks', 'graceful_shutdown'],
	service_label: 'Agent',
	hostname: 'prod-host',
	is_embedded: false,
	ip_address: '10.0.0.1',
	status: 'approved',
	client_version: '1.2.0',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	cert_lifetime_hours: null,
	yielded_to: null
};

const pendingAgent: ServiceResponse = {
	...approvedAgent,
	id: 'svc-002',
	friendly_name: 'pending-agent',
	status: 'pending'
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Services Page', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the page heading when a user is logged in', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Services')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('renders a service row after a successful API response', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('prod-agent')).toBeInTheDocument());
		expect(screen.getByText('prod-host')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
	});

	it('shows the empty-state message when the service list is empty', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText(/No services registered yet/)).toBeInTheDocument());
	});

	it('shows an error message and a Retry button when the API call fails', async () => {
		vi.mocked(api.getServices).mockRejectedValue(new Error('Connection refused'));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Connection refused')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="callout"]')).toBeInTheDocument();
	});

	it('renders nothing when no user is logged in', () => {
		vi.mocked(auth.getUser).mockReturnValue(null);
		render(ServicesPage);
		expect(screen.queryByText('Services')).not.toBeInTheDocument();
	});

	it('calls getServices with the software_discovery capability when the Agents filter button is clicked', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		// Wait for the initial load triggered by $effect
		await waitFor(() => expect(vi.mocked(api.getServices)).toHaveBeenCalledTimes(1));

		fireEvent.click(screen.getByRole('button', { name: 'Agents' }));

		await waitFor(() =>
			expect(vi.mocked(api.getServices)).toHaveBeenCalledWith(
				expect.objectContaining({ capability: 'software_discovery' })
			)
		);
	});

	it('displays the Pending status badge for a pending service', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([{ ...approvedAgent, status: 'pending' }]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Pending')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('displays the Approved status badge for an approved service', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Approved')).toBeInTheDocument());
	});

	it('shows embedded badges and hides delete for embedded services', async () => {
		vi.mocked(api.getServices).mockResolvedValue(
			makePage([
				{
					...approvedAgent,
					id: 'svc-embedded',
					friendly_name: 'embedded-agent',
					is_embedded: true,
					yielded_to: ['00000000-0000-0000-0000-000000000123']
				}
			])
		);
		render(ServicesPage);

		await waitFor(() => expect(screen.getByText('embedded-agent')).toBeInTheDocument());
		expect(screen.getByText('Embedded')).toBeInTheDocument();
		expect(screen.getByText('Yielded (1)')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge-stack"]')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: /actions for embedded-agent/i }));
		expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument();
		expect(screen.queryByRole('menuitem', { name: /delete/i })).not.toBeInTheDocument();
	});

	describe('capability filter chips', () => {
		beforeEach(() => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		});

		it('All Services chip is active by default — carries accent/bg-hover fragments', async () => {
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'All Services' })).toBeInTheDocument());
			const allChip = screen.getByRole('button', { name: 'All Services' });
			expect(allChip.className).toContain('text-[var(--accent)]');
			expect(allChip.className).toContain('bg-[var(--bg-hover)]');
		});

		it('inactive chips carry no accent/bg-hover fragments', async () => {
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Agents' })).toBeInTheDocument());
			for (const label of ['Agents', 'SSH Agents']) {
				const chip = screen.getByRole('button', { name: label });
				expect(chip.className).not.toContain('text-[var(--accent)]');
				expect(chip.className).not.toContain('bg-[var(--bg-hover)]');
			}
		});

		it('clicking Agents chip makes it active and deactivates All Services', async () => {
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Agents' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'Agents' }));
			await waitFor(() => {
				const agentsChip = screen.getByRole('button', { name: 'Agents' });
				expect(agentsChip.className).toContain('text-[var(--accent)]');
				expect(agentsChip.className).toContain('bg-[var(--bg-hover)]');
			});
			expect(screen.getByRole('button', { name: 'All Services' }).className).not.toContain('text-[var(--accent)]');
		});

		it('clicking SSH Agents chip makes it active', async () => {
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'SSH Agents' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'SSH Agents' }));
			await waitFor(() => {
				const sshChip = screen.getByRole('button', { name: 'SSH Agents' });
				expect(sshChip.className).toContain('text-[var(--accent)]');
			});
		});
	});

	describe('row ellipsis trigger', () => {
		it('renders variant="ghost" size="sm" — bg-transparent and h-[19px]', async () => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /actions for prod-agent/i })).toBeInTheDocument());
			const trigger = screen.getByRole('button', { name: /actions for prod-agent/i });
			expect(trigger.className).toContain('bg-transparent');
			expect(trigger.className).toContain('h-[19px]');
		});

		it('aria-label matches "Actions for {friendly_name}" including space in name', async () => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([{ ...approvedAgent, friendly_name: 'my prod agent' }]));
			render(ServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: 'Actions for my prod agent' })).toBeInTheDocument()
			);
		});

		it('clicking the trigger opens the context menu', async () => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /actions for prod-agent/i })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: /actions for prod-agent/i }));
			await waitFor(() => expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument());
		});
	});

	describe('Retry button', () => {
		it('renders variant="primary" (md size) in error state', async () => {
			vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			const retryBtn = screen.getByRole('button', { name: /retry/i });
			expect(retryBtn.className).toContain('h-[23px]');
			expect(retryBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		});

		it('sets aria-busy="true" during fetch and clears after rejection', async () => {
			vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			let resolveReject!: () => void;
			vi.mocked(api.getServices).mockReturnValue(
				new Promise<never>((_, reject) => {
					resolveReject = () => reject(new Error('still failing'));
				})
			);
			fireEvent.click(screen.getByRole('button', { name: /retry/i }));
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toHaveAttribute('aria-busy', 'true'));
			resolveReject();
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).not.toHaveAttribute('aria-busy'));
		});

		it('clears aria-busy after successful retry and hides the Retry button', async () => {
			vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
			fireEvent.click(screen.getByRole('button', { name: /retry/i }));
			await waitFor(() => expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument());
			expect(screen.getByText('prod-agent')).toBeInTheDocument();
		});
	});

	describe('Merge modal footer', () => {
		beforeEach(() => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([pendingAgent]));
		});

		async function openMergeModal() {
			render(ServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: /actions for pending-agent/i })).toBeInTheDocument()
			);
			fireEvent.click(screen.getByRole('button', { name: /actions for pending-agent/i }));
			await waitFor(() => expect(screen.getByText('Merge Into...')).toBeInTheDocument());
			fireEvent.click(screen.getByText('Merge Into...'));
			await waitFor(() => expect(screen.getByText('Merge Service')).toBeInTheDocument());
		}

		it('Cancel renders variant="secondary"', async () => {
			await openMergeModal();
			const cancelBtn = screen.getByRole('button', { name: /cancel/i });
			expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
			expect(cancelBtn.className).toContain('border');
		});

		it('Merge submit renders variant="primary" with static "Merge" label', async () => {
			await openMergeModal();
			const mergeBtn = screen.getByRole('button', { name: /^merge$/i });
			expect(mergeBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
			expect(mergeBtn.textContent).not.toContain('Merging');
		});

		it('Merge submit is disabled when no target is selected', async () => {
			await openMergeModal();
			expect(screen.getByRole('button', { name: /^merge$/i })).toBeDisabled();
		});

		it('loading=true sets aria-busy and disables the Merge submit', async () => {
			vi.mocked(api.mergeService).mockReturnValue(new Promise(() => {}));
			vi.mocked(api.getServices).mockResolvedValue(
				makePage([pendingAgent, { ...approvedAgent, id: 'svc-target', capabilities: ['software_discovery'] }])
			);
			await openMergeModal();
			const select = document.querySelector('select') as HTMLSelectElement;
			fireEvent.change(select, { target: { value: 'svc-target' } });
			await waitFor(() => expect(screen.getByRole('button', { name: /^merge$/i })).not.toBeDisabled());
			fireEvent.click(screen.getByRole('button', { name: /^merge$/i }));
			await waitFor(() =>
				expect(screen.getByRole('button', { name: /^merge$/i })).toHaveAttribute('aria-busy', 'true')
			);
			expect(screen.getByRole('button', { name: /^merge$/i })).toBeDisabled();
		});
	});

	describe('Ping modal footer (services)', () => {
		beforeEach(() => {
			vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
		});

		async function openPingModal() {
			render(ServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /actions for prod-agent/i })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: /actions for prod-agent/i }));
			await waitFor(() => expect(screen.getByText('Edit Ping Interval')).toBeInTheDocument());
			fireEvent.click(screen.getByText('Edit Ping Interval'));
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Edit Ping Interval' })).toBeInTheDocument());
		}

		it('Cancel renders variant="secondary"', async () => {
			await openPingModal();
			const cancelBtn = screen.getByRole('button', { name: /cancel/i });
			expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
		});

		it('Save renders variant="primary" with static "Save" label', async () => {
			await openPingModal();
			const saveBtn = screen.getByRole('button', { name: /^save$/i });
			expect(saveBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
			expect(saveBtn.textContent).not.toContain('Saving');
		});

		it('Save shows aria-busy during submit and "Saving..." text never appears', async () => {
			vi.mocked(api.updateService).mockReturnValue(new Promise(() => {}));
			await openPingModal();
			fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
			await waitFor(() => expect(screen.getByRole('button', { name: /^save$/i })).toHaveAttribute('aria-busy', 'true'));
			expect(document.body.textContent).not.toContain('Saving...');
		});
	});

	it('ContextMenuItem entries are not wrapped in <Button> (scope guard for #3k)', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([pendingAgent]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByRole('button', { name: /actions for pending-agent/i })).toBeInTheDocument());
		fireEvent.click(screen.getByRole('button', { name: /actions for pending-agent/i }));
		await waitFor(() => expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument());
		const menuItems = document.querySelectorAll('[data-ui="context-menu-item"]');
		expect(menuItems.length).toBeGreaterThan(0);
		for (const item of menuItems) {
			expect(item.closest('button[class*="h-[23px]"]')).toBeNull();
			expect(item.closest('button[class*="h-[19px]"]')).toBeNull();
		}
	});
});

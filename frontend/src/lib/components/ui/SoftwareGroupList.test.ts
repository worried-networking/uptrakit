import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import SoftwareGroupList from './SoftwareGroupList.svelte';
import type { SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse } from '$lib/types';

afterEach(() => {
	cleanup();
});

function makeItem(id: string, hostCount: number): SoftwareItemResponse {
	return {
		id,
		name: `Item ${id}`,
		plugins: ['generic_shell'],
		featured: false,
		last_checked_at: null,
		host_count: hostCount,
		installed_version: null,
		installed_display_version: null,
		latest_version: null,
		latest_release_metadata: null,
		update_available: false,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null
	};
}

function makeHost(rowId: string, hostId: string): SoftwareItemHostSummary {
	return {
		id: rowId,
		host_id: hostId,
		hostname: `host-${hostId}`,
		friendly_name: `host-${hostId}`,
		qualifier: null,
		installed_version: '1.0.0',
		installed_version_detected_at: '2024-01-01T00:00:00Z',
		installed_display_version: null,
		latest_version: null,
		latest_release_metadata: null,
		update_available: false,
		active_update_history_id: null,
		last_updated_at: null,
		linked_at: '2024-01-01T00:00:00Z',
		plugins: []
	};
}

function makeDetail(item: SoftwareItemResponse, hosts: SoftwareItemHostSummary[]): SoftwareItemDetailResponse {
	return { ...item, hosts };
}

function makeProps(
	overrides: Partial<{
		items: SoftwareItemResponse[];
		itemDetailsById: SvelteMap<string, SoftwareItemDetailResponse>;
		itemDetailLoadingIds: SvelteSet<string>;
		collapsedGroupIds: SvelteSet<string>;
		expandedOverflowGroupIds: SvelteSet<string>;
	}> = {}
) {
	return {
		items: overrides.items ?? [],
		itemDetailsById: overrides.itemDetailsById ?? new SvelteMap(),
		itemDetailLoadingIds: overrides.itemDetailLoadingIds ?? new SvelteSet(),
		collapsedGroupIds: overrides.collapsedGroupIds ?? new SvelteSet(),
		expandedOverflowGroupIds: overrides.expandedOverflowGroupIds ?? new SvelteSet(),
		batchSelectedIds: new SvelteSet<string>(),
		canManage: false,
		canTriggerUpdates: false,
		pluginTypeNames: new Map<string, string>(),
		totalItems: 0,
		currentPage: 1,
		totalPages: 1,
		onToggleGroup: vi.fn(),
		onToggleOverflow: vi.fn(),
		onToggleBatch: vi.fn(),
		onOpenMenu: vi.fn(),
		onOpenUpdateModal: vi.fn(),
		onPageChange: vi.fn(),
		onToggleFeatured: vi.fn()
	};
}

describe('SoftwareGroupList — zebra rows', () => {
	it('alternates bg on single-host item headers: idx=0 transparent, idx=1 raised, idx=2 transparent', () => {
		const itemA = makeItem('a', 1);
		const itemB = makeItem('b', 1);
		const itemC = makeItem('c', 1);
		const detailsById = new SvelteMap([
			['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])],
			['b', makeDetail(itemB, [makeHost('row-b1', 'h-b1')])],
			['c', makeDetail(itemC, [makeHost('row-c1', 'h-c1')])]
		]);

		render(SoftwareGroupList, makeProps({ items: [itemA, itemB, itemC], itemDetailsById: detailsById }));

		const headerA = screen.getByTestId('software-group-header-a');
		const headerB = screen.getByTestId('software-group-header-b');
		const headerC = screen.getByTestId('software-group-header-c');

		expect(headerA.className).not.toContain('bg-[var(--bg-raised)]'); // idx 0: transparent
		expect(headerB.className).toContain('bg-[var(--bg-raised)]'); // idx 1: raised
		expect(headerC.className).not.toContain('bg-[var(--bg-raised)]'); // idx 2: transparent
	});

	it('all header rows have hover:bg-[var(--bg-hover)] and transition classes', () => {
		const itemA = makeItem('a', 1);
		const detailsById = new SvelteMap([['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])]]);

		render(SoftwareGroupList, makeProps({ items: [itemA], itemDetailsById: detailsById }));

		const headerA = screen.getByTestId('software-group-header-a');
		expect(headerA.className).toContain('hover:bg-[var(--bg-hover)]');
		expect(headerA.className).toContain('transition-[background,border-color,color]');
		expect(headerA.className).toContain('duration-fast');
	});

	it('host sub-rows continue flat index: itemA=0, itemB_header=1, host1=2, host2=3, itemC=4', () => {
		const itemA = makeItem('a', 1);
		const itemB = makeItem('b', 2);
		const itemC = makeItem('c', 1);
		const host1 = makeHost('row-h1', 'hid1');
		const host2 = makeHost('row-h2', 'hid2');
		const detailsById = new SvelteMap([
			['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])],
			['b', makeDetail(itemB, [host1, host2])],
			['c', makeDetail(itemC, [makeHost('row-c1', 'h-c1')])]
		]);

		render(SoftwareGroupList, makeProps({ items: [itemA, itemB, itemC], itemDetailsById: detailsById }));

		const headerA = screen.getByTestId('software-group-header-a');
		const headerB = screen.getByTestId('software-group-header-b');
		const hostRow1 = screen.getByTestId('software-host-row-row-h1');
		const hostRow2 = screen.getByTestId('software-host-row-row-h2');
		const headerC = screen.getByTestId('software-group-header-c');

		expect(headerA.className).not.toContain('bg-[var(--bg-raised)]'); // idx 0
		expect(headerB.className).toContain('bg-[var(--bg-raised)]'); // idx 1
		expect(hostRow1.className).not.toContain('bg-[var(--bg-raised)]'); // idx 2
		expect(hostRow2.className).toContain('bg-[var(--bg-raised)]'); // idx 3
		expect(headerC.className).not.toContain('bg-[var(--bg-raised)]'); // idx 4
	});

	it('host sub-rows have hover:bg-[var(--bg-hover)]', () => {
		const itemB = makeItem('b', 2);
		const host1 = makeHost('row-h1', 'hid1');
		const host2 = makeHost('row-h2', 'hid2');
		const detailsById = new SvelteMap([['b', makeDetail(itemB, [host1, host2])]]);

		render(SoftwareGroupList, makeProps({ items: [itemB], itemDetailsById: detailsById }));

		const hostRow1 = screen.getByTestId('software-host-row-row-h1');
		expect(hostRow1.className).toContain('hover:bg-[var(--bg-hover)]');
	});

	it('collapsing a multi-host item re-stripes downstream headers', () => {
		// With B expanded (3 hosts): A=0(T), B=1(R), h1=2, h2=3, h3=4, C=5(R), D=6(T)
		// With B collapsed:          A=0(T), B=1(R),                   C=2(T), D=3(R)
		// C: raised → transparent. D: transparent → raised.
		const itemA = makeItem('a', 1);
		const itemB = makeItem('b', 3);
		const itemC = makeItem('c', 1);
		const itemD = makeItem('d', 1);
		const host1 = makeHost('row-h1', 'hid1');
		const host2 = makeHost('row-h2', 'hid2');
		const host3 = makeHost('row-h3', 'hid3');
		const detailsById = new SvelteMap([
			['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])],
			['b', makeDetail(itemB, [host1, host2, host3])],
			['c', makeDetail(itemC, [makeHost('row-c1', 'h-c1')])],
			['d', makeDetail(itemD, [makeHost('row-d1', 'h-d1')])]
		]);
		const items = [itemA, itemB, itemC, itemD];

		// B expanded
		const { rerender } = render(
			SoftwareGroupList,
			makeProps({
				items,
				itemDetailsById: detailsById,
				collapsedGroupIds: new SvelteSet()
			})
		);
		expect(screen.getByTestId('software-group-header-c').className).toContain('bg-[var(--bg-raised)]'); // idx 5
		expect(screen.getByTestId('software-group-header-d').className).not.toContain('bg-[var(--bg-raised)]'); // idx 6

		// B collapsed
		rerender(
			makeProps({
				items,
				itemDetailsById: detailsById,
				collapsedGroupIds: new SvelteSet(['b'])
			})
		);
		expect(screen.getByTestId('software-group-header-c').className).not.toContain('bg-[var(--bg-raised)]'); // idx 2
		expect(screen.getByTestId('software-group-header-d').className).toContain('bg-[var(--bg-raised)]'); // idx 3
	});

	it('expanding overflow re-stripes: 4th host appears at correct index, overflow row disappears', () => {
		// A=0(T), B_header=1(R), h1=2(T), h2=3(R), h3=4(T), overflow=5(R) — before expand
		// A=0(T), B_header=1(R), h1=2(T), h2=3(R), h3=4(T), h4=5(R)      — after expand
		const itemA = makeItem('a', 1);
		const itemB = makeItem('b', 4);
		const host1 = makeHost('row-h1', 'hid1');
		const host2 = makeHost('row-h2', 'hid2');
		const host3 = makeHost('row-h3', 'hid3');
		const host4 = makeHost('row-h4', 'hid4');
		const detailsById = new SvelteMap([
			['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])],
			['b', makeDetail(itemB, [host1, host2, host3, host4])]
		]);
		const items = [itemA, itemB];

		// Overflow not expanded: 3 hosts visible, host4 hidden
		const { rerender } = render(
			SoftwareGroupList,
			makeProps({ items, itemDetailsById: detailsById, expandedOverflowGroupIds: new SvelteSet() })
		);
		expect(screen.queryByTestId('software-host-row-row-h4')).toBeNull(); // host4 hidden by overflow

		// Overflow expanded: all 4 hosts visible, stripes re-number
		rerender(makeProps({ items, itemDetailsById: detailsById, expandedOverflowGroupIds: new SvelteSet(['b']) }));
		const hostRow3 = screen.getByTestId('software-host-row-row-h3');
		const hostRow4 = screen.getByTestId('software-host-row-row-h4');
		expect(hostRow3.className).not.toContain('bg-[var(--bg-raised)]'); // idx 4: transparent
		expect(hostRow4.className).toContain('bg-[var(--bg-raised)]'); // idx 5: raised (took overflow slot)
	});

	it('mobile cards alternate bg: idx=0 transparent, idx=1 raised', () => {
		const itemA = makeItem('a', 1);
		const itemB = makeItem('b', 1);
		const detailsById = new SvelteMap([
			['a', makeDetail(itemA, [makeHost('row-a1', 'h-a1')])],
			['b', makeDetail(itemB, [makeHost('row-b1', 'h-b1')])]
		]);

		render(SoftwareGroupList, makeProps({ items: [itemA, itemB], itemDetailsById: detailsById }));

		const cardA = screen.getByTestId('software-group-mobile-a');
		const cardB = screen.getByTestId('software-group-mobile-b');

		expect(cardA.className).not.toContain('bg-[var(--bg-raised)]');
		expect(cardB.className).toContain('bg-[var(--bg-raised)]');
		expect(cardA.className).toContain('hover:bg-[var(--bg-hover)]');
		expect(cardB.className).toContain('hover:bg-[var(--bg-hover)]');
	});
});

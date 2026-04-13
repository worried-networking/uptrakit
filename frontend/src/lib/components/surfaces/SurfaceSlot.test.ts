import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import SurfaceSlot from './SurfaceSlot.svelte';
import { getSurfaceDescriptorRenderKey } from '$lib/surfaces/interactions';
import type { SurfaceResponse } from '$lib/surfaces/contract';

afterEach(() => {
	cleanup();
});

function makeSurface(label: string): SurfaceResponse {
	return {
		surface_id: 'dup.surface',
		label,
		priority: 100,
		slot: 'extension.page',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'key_value', data_source_id: 'info' },
		provider_count: 1
	};
}

describe('SurfaceSlot', () => {
	it('resolves variant maps by descriptor key before surface_id fallback', () => {
		const first = makeSurface('First');
		const second = makeSurface('Second');
		const firstKey = getSurfaceDescriptorRenderKey(first);
		const secondKey = getSurfaceDescriptorRenderKey(second);

		render(SurfaceSlot, {
			slot: 'extension.page',
			surfaces: [first, second],
			dataBySurface: {
				[firstKey]: { info: { variant: 'first' } },
				[secondKey]: { info: { variant: 'second' } },
				[first.surface_id]: { info: { variant: 'fallback' } }
			}
		});

		expect(screen.getByText('first')).toBeInTheDocument();
		expect(screen.getByText('second')).toBeInTheDocument();
		expect(screen.queryByText('fallback')).not.toBeInTheDocument();
	});

	it('falls back to surface_id keyed state when descriptor key is absent', () => {
		const first = makeSurface('First');
		const second = makeSurface('Second');

		render(SurfaceSlot, {
			slot: 'extension.page',
			surfaces: [first, second],
			dataBySurface: {
				[first.surface_id]: { info: { variant: 'fallback' } }
			}
		});

		expect(screen.getAllByText('fallback')).toHaveLength(2);
	});
});

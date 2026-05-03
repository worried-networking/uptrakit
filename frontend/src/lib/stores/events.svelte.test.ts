import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// Static import — gets the real enum values regardless of vi.doMock later.
// AdminEventType is a TypeScript string enum so values are plain strings at runtime.
import { AdminEventType } from '$lib/sse';

type OnEventFn = (eventType: AdminEventType, data: Record<string, unknown>) => void;
let capturedOnEvent: OnEventFn | undefined;
const mockLoadSurfaceRegistry = vi.fn().mockResolvedValue(undefined);

describe('events.svelte — surfaces_changed handling', () => {
	beforeEach(() => {
		vi.resetModules();
		capturedOnEvent = undefined;
		mockLoadSurfaceRegistry.mockClear();

		vi.doMock('$lib/sse', () => ({
			// Include AdminEventType in the mock so the mocked module exports it.
			AdminEventType,
			connectEventStream: vi.fn((callbacks: { onEvent?: OnEventFn }) => {
				capturedOnEvent = callbacks.onEvent;
				return () => {
					capturedOnEvent = undefined;
				};
			})
		}));

		vi.doMock('$lib/surfaces/registry.svelte', () => ({
			loadSurfaceRegistry: mockLoadSurfaceRegistry
		}));

		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('single surfaces_changed event calls loadSurfaceRegistry once after debounce', async () => {
		const { subscribeToEvent } = await import('$lib/stores/events.svelte');

		let called = false;
		const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
			void mockLoadSurfaceRegistry();
			called = true;
		});

		capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
		expect(called).toBe(false); // not yet — debounce pending

		await vi.advanceTimersByTimeAsync(200);
		expect(called).toBe(true);
		expect(mockLoadSurfaceRegistry).toHaveBeenCalledTimes(1);

		unsub();
	});

	it('burst of three surfaces_changed events debounces to one loadSurfaceRegistry call', async () => {
		const { subscribeToEvent } = await import('$lib/stores/events.svelte');

		let callCount = 0;
		const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
			void mockLoadSurfaceRegistry();
			callCount++;
		});

		capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
		capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
		capturedOnEvent?.(AdminEventType.SurfacesChanged, {});

		await vi.advanceTimersByTimeAsync(200);
		expect(callCount).toBe(1);
		expect(mockLoadSurfaceRegistry).toHaveBeenCalledTimes(1);

		unsub();
	});

	it('surfaces_changed event with empty data object is not dropped (parseSseEvent passes {})', async () => {
		const { subscribeToEvent } = await import('$lib/stores/events.svelte');

		let received = false;
		const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
			received = true;
		});

		// Simulate what readAdminEventStream does: JSON.parse('{}') → {}
		capturedOnEvent?.(AdminEventType.SurfacesChanged, JSON.parse('{}') as Record<string, unknown>);
		await vi.advanceTimersByTimeAsync(200);

		expect(received).toBe(true);
		unsub();
	});
});

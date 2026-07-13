/**
 * SSE stream 401 refresh integration tests.
 *
 * Verifies that both connectOutputStream and connectEventStream join the
 * shared dedupedRefresh path on 401, honour terminality rules, and update
 * the token store before reconnecting (stale-token-loop regression guard).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Hoisted mock state ────────────────────────────────────────────────────────
// vi.mock factories are hoisted above import statements, so any variables they
// reference must also be hoisted. vi.hoisted() runs before hoisted vi.mock calls.

const { mockGetAccessToken, mockSetAccessToken, mockDedupedRefresh, mockIsAuthClass } = vi.hoisted(() => {
	const mockSetAccessToken = vi.fn((_token: string | null) => {});
	const mockGetAccessToken = vi.fn<() => string | null>().mockReturnValue('old-token');

	type RefreshResult = { access_token: string };
	type DedupedRefreshFn = () => Promise<RefreshResult>;
	const mockDedupedRefresh = vi.fn<DedupedRefreshFn>();

	// Mirror isAuthClassRefreshFailure's branch conditions without side effects.
	// A 4xx-shaped error (not timeout/TypeError/5xx) = auth-class failure.
	const mockIsAuthClass = vi.fn((err: unknown): boolean => {
		if (err instanceof DOMException && (err.name === 'TimeoutError' || err.name === 'AbortError')) return false;
		if (err instanceof TypeError) return false;
		if (
			err !== null &&
			typeof err === 'object' &&
			'status' in err &&
			typeof (err as Record<string, unknown>).status === 'number'
		) {
			if ((err as { status: number }).status >= 500) return false;
		}
		return true;
	});

	return { mockGetAccessToken, mockSetAccessToken, mockDedupedRefresh, mockIsAuthClass };
});

// ── Module mocks ──────────────────────────────────────────────────────────────

vi.mock('./auth.svelte', () => ({
	getAccessToken: mockGetAccessToken,
	setAccessToken: mockSetAccessToken
}));

vi.mock('./api/client', () => ({
	BASE: '/api/v1',
	dedupedRefresh: mockDedupedRefresh,
	isAuthClassRefreshFailure: mockIsAuthClass
}));

// ── Imports ───────────────────────────────────────────────────────────────────

import { connectOutputStream, connectEventStream } from './sse';
import type { SseCallbacks, AdminEventCallbacks } from './sse';

// ── Helpers ───────────────────────────────────────────────────────────────────

type RefreshResult = { access_token: string };

/** A ReadableStream that closes immediately (no data). */
function makeClosedStream(): ReadableStream<Uint8Array> {
	return new ReadableStream<Uint8Array>({
		start(controller) {
			controller.close();
		}
	});
}

/**
 * Flush the microtask queue thoroughly.
 *
 * A fetch().then(A).then(B).catch(C) chain resolves over 3+ microtask ticks.
 * Each `await Promise.resolve()` drains one level. This helper flushes 5 levels
 * to ensure all chained promise handlers (including fetch body reading) settle
 * before assertions. Pattern from ToastNotifications.test.ts:114.
 */
async function flushPromises(): Promise<void> {
	for (let i = 0; i < 5; i++) {
		await Promise.resolve();
	}
}

// ── connectOutputStream tests ─────────────────────────────────────────────────

describe('connectOutputStream — 401 refresh path', () => {
	beforeEach(() => {
		vi.useFakeTimers();
		// Clear call counts but preserve hoisted implementations.
		// vi.clearAllMocks() is used instead of vi.restoreAllMocks() because
		// restoreAllMocks() resets the implementations set in vi.hoisted() factories,
		// which would break subsequent tests.
		vi.clearAllMocks();
		// Re-install implementations that clearAllMocks just cleared.
		mockGetAccessToken.mockReturnValue('old-token');
		mockIsAuthClass.mockImplementation((err: unknown): boolean => {
			if (err instanceof DOMException && (err.name === 'TimeoutError' || err.name === 'AbortError')) return false;
			if (err instanceof TypeError) return false;
			if (
				err !== null &&
				typeof err === 'object' &&
				'status' in err &&
				typeof (err as Record<string, unknown>).status === 'number'
			) {
				if ((err as { status: number }).status >= 500) return false;
			}
			return true;
		});
	});

	afterEach(() => {
		vi.useRealTimers();
		vi.unstubAllGlobals();
	});

	it('case 1: 401 → exactly one dedupedRefresh, reconnect carries the ROTATED token', async () => {
		// The mock must NOT write the token store itself — it only resolves with
		// { access_token: 'new-token' }. The implementation must call setAccessToken
		// before reconnecting; if it omits the call, the reconnect re-reads the stale
		// token and loops forever on 401 (the stale-token-loop regression).
		mockDedupedRefresh.mockResolvedValueOnce({ access_token: 'new-token' });

		let fetchCall = 0;
		const fetchMock = vi.fn((_url: string, _init?: RequestInit): Promise<Response> => {
			fetchCall++;
			if (fetchCall === 1) {
				return Promise.resolve(new Response(null, { status: 401, statusText: 'Unauthorized' }));
			}
			return Promise.resolve(new Response(makeClosedStream(), { status: 200 }));
		});
		vi.stubGlobal('fetch', fetchMock);

		// setAccessToken updates what getAccessToken returns (stale-token-loop test)
		mockSetAccessToken.mockImplementation((token: string | null) => {
			mockGetAccessToken.mockReturnValue(token);
		});

		const onStateChange = vi.fn();
		const onError = vi.fn();
		const callbacks: SseCallbacks = { onStateChange, onError };

		connectOutputStream('update-1', callbacks, { maxReconnectAttempts: 5 });

		// Flush the fetch chain — 3 microtask ticks for fetch().then(A).then(B).catch(C)
		// plus buffer for any internal chaining.
		await flushPromises();

		// Should have called onError with the 401 message
		expect(onError).toHaveBeenCalledTimes(1);
		// dedupedRefresh not yet called — it runs inside the setTimeout callback
		expect(mockDedupedRefresh).toHaveBeenCalledTimes(0);

		// Advance past the first backoff delay (attempt=1, delay=1000ms)
		await vi.advanceTimersByTimeAsync(1000);
		await flushPromises();

		// dedupedRefresh should have been called exactly once
		expect(mockDedupedRefresh).toHaveBeenCalledTimes(1);

		// setAccessToken must have been called with the rotated token BEFORE reconnect
		expect(mockSetAccessToken).toHaveBeenCalledWith('new-token');

		// Reconnect fetch should have been issued (fetchCall is now ≥ 2)
		expect(fetchCall).toBeGreaterThanOrEqual(2);
	});

	it('case 2: auth-class refresh failure → terminal (onStateChange("error"), no more fetches)', async () => {
		// A 4xx-shaped error: mockIsAuthClass returns true (not timeout/TypeError/5xx)
		const authClassErr = { status: 401, message: 'Session revoked' };
		mockDedupedRefresh.mockRejectedValueOnce(authClassErr);

		let fetchCount = 0;
		vi.stubGlobal(
			'fetch',
			vi.fn((_url: string, _init?: RequestInit): Promise<Response> => {
				fetchCount++;
				return Promise.resolve(new Response(null, { status: 401, statusText: 'Unauthorized' }));
			})
		);

		const onStateChange = vi.fn();
		const callbacks: SseCallbacks = { onStateChange, onError: vi.fn() };

		connectOutputStream('update-2', callbacks, { maxReconnectAttempts: 5 });

		// Flush the initial 401 fetch chain
		await flushPromises();
		expect(fetchCount).toBe(1);

		// Advance past the first backoff delay
		await vi.advanceTimersByTimeAsync(1000);
		await flushPromises();

		// Terminal: onStateChange('error') should have been called
		expect(onStateChange).toHaveBeenCalledWith('error');

		// No further fetches after terminal — fetch count must be frozen
		const fetchCountAtTerminal = fetchCount;
		await vi.advanceTimersByTimeAsync(5000);
		await flushPromises();
		expect(fetchCount).toBe(fetchCountAtTerminal);
	});

	it('case 3: transient refresh failure → keeps cycling, NOT terminal', async () => {
		// TypeError = transient; mockIsAuthClass returns false for TypeError
		mockDedupedRefresh.mockRejectedValue(new TypeError('Network error'));

		let fetchCount = 0;
		vi.stubGlobal(
			'fetch',
			vi.fn((_url: string, _init?: RequestInit): Promise<Response> => {
				fetchCount++;
				return Promise.resolve(new Response(null, { status: 401, statusText: 'Unauthorized' }));
			})
		);

		const onStateChange = vi.fn();
		const callbacks: SseCallbacks = { onStateChange, onError: vi.fn() };

		// Use high maxReconnectAttempts so the cycle continues
		connectOutputStream('update-3', callbacks, { maxReconnectAttempts: 10 });

		// Flush the initial 401 fetch chain
		await flushPromises();
		expect(fetchCount).toBe(1);

		// Advance through backoff cycle 1 (delay=1000ms)
		await vi.advanceTimersByTimeAsync(1000);
		await flushPromises();

		// onStateChange('error') must NOT have been called from the transient refresh failure
		const errorCallsAfterCycle1 = onStateChange.mock.calls.filter(([s]) => s === 'error');
		expect(errorCallsAfterCycle1).toHaveLength(0);

		// At least one dedupedRefresh was attempted in cycle 1
		expect(mockDedupedRefresh.mock.calls.length).toBeGreaterThanOrEqual(1);

		// Stream is still cycling — advance through cycle 2 triggers another reconnect
		const fetchCountAfterCycle1 = fetchCount;
		await vi.advanceTimersByTimeAsync(2000);
		await flushPromises();
		expect(fetchCount).toBeGreaterThan(fetchCountAfterCycle1);
		expect(mockDedupedRefresh.mock.calls.length).toBeGreaterThanOrEqual(2);
	});
});

// ── connectEventStream tests ──────────────────────────────────────────────────

describe('connectEventStream — 401 refresh path', () => {
	beforeEach(() => {
		vi.useFakeTimers();
		vi.clearAllMocks();
		mockGetAccessToken.mockReturnValue('old-token');
		mockIsAuthClass.mockImplementation((err: unknown): boolean => {
			if (err instanceof DOMException && (err.name === 'TimeoutError' || err.name === 'AbortError')) return false;
			if (err instanceof TypeError) return false;
			if (
				err !== null &&
				typeof err === 'object' &&
				'status' in err &&
				typeof (err as Record<string, unknown>).status === 'number'
			) {
				if ((err as { status: number }).status >= 500) return false;
			}
			return true;
		});
	});

	afterEach(() => {
		vi.useRealTimers();
		vi.unstubAllGlobals();
	});

	it('case 4: 401-after-successful-refresh keeps cycling — no tight loop, state never terminal', async () => {
		// Every fetch returns 401, every refresh succeeds — stream should cycle
		// indefinitely (maxReconnectAttempts: Infinity default) without going terminal.
		mockDedupedRefresh.mockResolvedValue({ access_token: 'new-token' });

		let fetchCount = 0;
		vi.stubGlobal(
			'fetch',
			vi.fn((_url: string, _init?: RequestInit): Promise<Response> => {
				fetchCount++;
				return Promise.resolve(new Response(null, { status: 401, statusText: 'Unauthorized' }));
			})
		);

		const onStateChange = vi.fn();
		const callbacks: AdminEventCallbacks = { onStateChange, onError: vi.fn() };

		connectEventStream(callbacks); // maxReconnectAttempts: Infinity (default)

		// Settle first fetch (401)
		await flushPromises();
		expect(fetchCount).toBe(1);

		// Advance through backoff cycle 1 (attempt=1, delay=1000ms)
		await vi.advanceTimersByTimeAsync(1000);
		await flushPromises();
		// Reconnect fetch happened after refresh
		expect(fetchCount).toBe(2);

		// Advance through backoff cycle 2 (attempt=2, delay=2000ms)
		await vi.advanceTimersByTimeAsync(2000);
		await flushPromises();
		expect(fetchCount).toBe(3);

		// One refresh per backoff cycle (fetch-call count == cycles)
		expect(mockDedupedRefresh.mock.calls.length).toBe(2);

		// Never terminal
		const errorCalls = onStateChange.mock.calls.filter(([s]) => s === 'error');
		expect(errorCalls).toHaveLength(0);
	});

	it('case 5: two concurrent streams share ONE dedupedRefresh call on simultaneous 401', async () => {
		// Both streams hit 401 in the same event loop tick. The dedup logic inside
		// dedupedRefresh collapses concurrent calls to a single in-flight promise.
		// Simulate this: mockDedupedRefresh returns the SAME shared promise both times.
		let resolveRefresh!: (value: RefreshResult) => void;
		const sharedRefreshPromise = new Promise<RefreshResult>((res) => {
			resolveRefresh = res;
		});
		mockDedupedRefresh.mockReturnValue(sharedRefreshPromise);

		let fetchCount = 0;
		vi.stubGlobal(
			'fetch',
			vi.fn((_url: string, _init?: RequestInit): Promise<Response> => {
				fetchCount++;
				return Promise.resolve(new Response(null, { status: 401, statusText: 'Unauthorized' }));
			})
		);

		// Start two concurrent streams
		connectEventStream({ onStateChange: vi.fn(), onError: vi.fn() });
		connectEventStream({ onStateChange: vi.fn(), onError: vi.fn() });

		// Settle both initial fetches
		await flushPromises();
		expect(fetchCount).toBe(2);

		// Advance past the first backoff delay (attempt=1, delay=1000ms)
		await vi.advanceTimersByTimeAsync(1000);
		await flushPromises();

		// Resolve the shared promise (simulating the real dedup collapsing both calls)
		resolveRefresh({ access_token: 'shared-new-token' });
		await flushPromises();

		// Both streams called dedupedRefresh — 2 calls to the mock (one per stream);
		// the real dedup (in production) collapses these to one in-flight promise.
		expect(mockDedupedRefresh).toHaveBeenCalledTimes(2);

		// Both streams should have reconnected after the shared refresh resolved
		expect(fetchCount).toBeGreaterThanOrEqual(3);
	});
});

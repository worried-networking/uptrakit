/**
 * SSE (Server-Sent Events) connection utility.
 *
 * Uses `fetch()` with streaming `ReadableStream` instead of `EventSource` to
 * support custom headers (Authorization: Bearer).
 */

import { getAccessToken, setAccessToken } from './auth.svelte';
import { BASE, dedupedRefresh, isAuthClassRefreshFailure } from './api/client';

/** Module-local marker for a 401 stream response (repo idiom: Error
 *  subclasses carry extra meaning — ApiError, RefreshError). Not exported. */
class UnauthorizedError extends Error {}

/** Connection states for the SSE stream. */
export type SseConnectionState = 'connecting' | 'streaming' | 'completed' | 'error' | 'disconnected';

/** A single output line event from the SSE stream. */
export interface OutputLineEvent {
	id: string;
	text: string;
	stream: string;
	timestamp: string;
	seq: number;
}

/** The update completed event from the SSE stream. */
export interface CompletedEvent {
	status: string;
	error: string | null;
}

/** Callbacks for the SSE connection. */
export interface SseCallbacks {
	onOutput?: (line: OutputLineEvent) => void;
	onCompleted?: (event: CompletedEvent) => void;
	onStateChange?: (state: SseConnectionState) => void;
	onError?: (error: string) => void;
}

/** Options for the SSE connection. */
export interface SseOptions {
	/** Maximum reconnection attempts (default: 5). */
	maxReconnectAttempts?: number;
}

/**
 * Connect to the update output SSE stream.
 *
 * Returns a `disconnect` function that cleanly closes the connection.
 */
export function connectOutputStream(
	updateHistoryId: string,
	callbacks: SseCallbacks,
	options?: SseOptions
): () => void {
	const maxAttempts = options?.maxReconnectAttempts ?? 5;
	let abortController: AbortController | null = null;
	let disconnected = false;
	let attempt = 0;

	function connect() {
		if (disconnected) return;

		abortController = new AbortController();
		callbacks.onStateChange?.('connecting');

		const token = getAccessToken();
		const headers: Record<string, string> = {
			Accept: 'text/event-stream'
		};
		if (token) {
			headers['Authorization'] = `Bearer ${token}`;
		}

		fetch(`${BASE}/update-history/${updateHistoryId}/output/stream`, {
			headers,
			credentials: 'same-origin',
			signal: abortController.signal
		})
			.then((response) => {
				if (!response.ok) {
					if (response.status === 401) {
						throw new UnauthorizedError('unauthorized');
					}
					throw new Error(`HTTP ${response.status}: ${response.statusText}`);
				}
				if (!response.body) {
					throw new Error('Response body is null');
				}

				callbacks.onStateChange?.('streaming');
				attempt = 0; // Reset on successful connection.

				return readSseStream(response.body, callbacks);
			})
			.then(() => {
				// Stream ended normally (server closed connection).
				if (!disconnected) {
					callbacks.onStateChange?.('disconnected');
				}
			})
			.catch((err: unknown) => {
				if (disconnected || (err instanceof DOMException && err.name === 'AbortError')) {
					return; // Clean disconnect.
				}

				const unauthorized = err instanceof UnauthorizedError;
				const message = err instanceof Error ? err.message : 'Connection failed';
				callbacks.onError?.(message);

				// Attempt reconnection with exponential backoff.
				attempt++;
				if (attempt <= maxAttempts) {
					const delay = Math.min(1000 * Math.pow(2, attempt - 1), 30000);
					callbacks.onStateChange?.('connecting');
					setTimeout(() => {
						if (disconnected) return;
						if (unauthorized) {
							// Session refresh through the shared deduped path. Terminality
							// keys on the REFRESH, not the reconnect: an auth-class refresh
							// failure (mapRefreshFailure's 4xx branch) is the true "session
							// revoked" signal and terminates the stream. A transient refresh
							// failure (timeout/TypeError/5xx) is NOT terminal — it re-enters
							// this same backed-off cycle. A 401 on the reconnect itself
							// (after a successful refresh) is ambiguous (propagation lag,
							// multi-tab rotation) and also simply re-enters this cycle
							// (falls into the `unauthorized` branch again next attempt).
							//
							// Note: dedupedRefresh dedup is per-tab (per JS context) — N tabs
							// waking from expiry issue N refreshes; correct (refresh-token
							// cookie) and pre-existing; cross-tab coordination is out of scope.
							void dedupedRefresh().then(
								(refreshed) => {
									// dedupedRefresh returns the result but does NOT write the
									// token store — both existing consumers set it themselves
									// (client.ts refreshAndRetry, raw.ts). Without this line the
									// reconnect re-reads the STALE token and loops on 401.
									setAccessToken(refreshed.access_token);
									connect();
								},
								(refreshErr) => {
									if (isAuthClassRefreshFailure(refreshErr)) {
										callbacks.onStateChange?.('error');
										return;
									}
									// Transient refresh failure: not terminal. Re-enter the
									// normal reconnect cycle (attempt already incremented above;
									// the next catch's backoff/refresh handles retry).
									connect();
								}
							);
						} else {
							connect();
						}
					}, delay);
				} else {
					callbacks.onStateChange?.('error');
				}
			});
	}

	connect();

	return () => {
		disconnected = true;
		abortController?.abort();
		callbacks.onStateChange?.('disconnected');
	};
}

/**
 * Parse an SSE stream from a ReadableStream<Uint8Array>.
 *
 * Resolves when the stream ends or when a `completed` event is received.
 */
async function readSseStream(body: ReadableStream<Uint8Array>, callbacks: SseCallbacks): Promise<void> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';

	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;

			buffer += decoder.decode(value, { stream: true });

			// Split on event boundaries (double newline).
			let boundaryIndex: number;
			while ((boundaryIndex = buffer.indexOf('\n\n')) !== -1) {
				const eventText = buffer.slice(0, boundaryIndex);
				buffer = buffer.slice(boundaryIndex + 2);

				const event = parseSseEvent(eventText);
				if (!event) continue;

				if (event.type === 'output' && event.data) {
					try {
						const line: OutputLineEvent = JSON.parse(event.data);
						callbacks.onOutput?.(line);
					} catch {
						// Skip malformed events.
					}
				} else if (event.type === 'completed' && event.data) {
					try {
						const completed: CompletedEvent = JSON.parse(event.data);
						callbacks.onCompleted?.(completed);
						callbacks.onStateChange?.('completed');
					} catch {
						// Skip malformed events.
					}
					return; // Done.
				}
			}
		}
	} finally {
		reader.releaseLock();
	}
}

interface ParsedSseEvent {
	type: string;
	data: string;
}

// ── Admin event stream ────────────────────────────────────────────────

/** Known admin event types pushed by `GET /api/v1/events/stream`. */
export enum AdminEventType {
	HostUpdated = 'host_updated',
	HostCreated = 'host_created',
	HostDeleted = 'host_deleted',
	ServiceStatusChanged = 'service_status_changed',
	SoftwareItemUpdated = 'software_item_updated',
	SoftwareItemCreated = 'software_item_created',
	VersionCheckCompleted = 'version_check_completed',
	UpdateTriggered = 'update_triggered',
	UpdateProtectionStarted = 'update_protection_started',
	UpdateStarted = 'update_started',
	UpdateCompleted = 'update_completed',
	DiscoveryCompleted = 'discovery_completed',
	BatchUpdateCompleted = 'batch_update_completed',
	SystemServiceStatusChanged = 'system_service_status_changed',
	SchedulerTaskCompleted = 'scheduler_task_completed',
	HostTagCreated = 'host_tag_created',
	HostTagUpdated = 'host_tag_updated',
	HostTagDeleted = 'host_tag_deleted',
	HostTagsChanged = 'host_tags_changed',
	SurfacesChanged = 'surfaces_changed'
}

/** Callbacks for the admin event SSE connection. */
export interface AdminEventCallbacks {
	onEvent?: (eventType: AdminEventType, data: Record<string, unknown>) => void;
	onStateChange?: (state: SseConnectionState) => void;
	onError?: (error: string) => void;
}

/**
 * Connect to the admin events SSE stream.
 *
 * Returns a `disconnect` function that cleanly closes the connection.
 * The stream reconnects automatically with exponential backoff on errors.
 * Unlike the output stream, there is no terminal event — the connection
 * stays open until explicitly disconnected or all reconnection attempts
 * are exhausted.
 */
export function connectEventStream(callbacks: AdminEventCallbacks, options?: SseOptions): () => void {
	const maxAttempts = options?.maxReconnectAttempts ?? Infinity;
	let abortController: AbortController | null = null;
	let disconnected = false;
	let attempt = 0;

	function connect() {
		if (disconnected) return;

		abortController = new AbortController();
		callbacks.onStateChange?.('connecting');

		const token = getAccessToken();
		const headers: Record<string, string> = {
			Accept: 'text/event-stream'
		};
		if (token) {
			headers['Authorization'] = `Bearer ${token}`;
		}

		fetch(`${BASE}/events/stream`, {
			headers,
			credentials: 'same-origin',
			signal: abortController.signal
		})
			.then((response) => {
				if (!response.ok) {
					if (response.status === 401) {
						throw new UnauthorizedError('unauthorized');
					}
					throw new Error(`HTTP ${response.status}: ${response.statusText}`);
				}
				if (!response.body) {
					throw new Error('Response body is null');
				}

				callbacks.onStateChange?.('streaming');
				attempt = 0;

				return readAdminEventStream(response.body, callbacks);
			})
			.then(() => {
				// Stream ended normally (server closed). Reconnect.
				// This is NOT a 401 path — leave it unchanged.
				if (!disconnected) {
					attempt++;
					const delay = Math.min(1000 * Math.pow(2, attempt - 1), 30000);
					callbacks.onStateChange?.('connecting');
					setTimeout(connect, delay);
				}
			})
			.catch((err: unknown) => {
				if (disconnected || (err instanceof DOMException && err.name === 'AbortError')) {
					return;
				}

				const unauthorized = err instanceof UnauthorizedError;
				const message = err instanceof Error ? err.message : 'Connection failed';
				callbacks.onError?.(message);

				// Note: a dead-but-cycling events stream has no user-facing signal
				// (accepted gap — follow-up material, not in scope for this fix).
				attempt++;
				if (attempt <= maxAttempts) {
					const delay = Math.min(1000 * Math.pow(2, attempt - 1), 30000);
					callbacks.onStateChange?.('connecting');
					setTimeout(() => {
						if (disconnected) return;
						if (unauthorized) {
							// Session refresh through the shared deduped path. Terminality
							// keys on the REFRESH, not the reconnect: an auth-class refresh
							// failure (mapRefreshFailure's 4xx branch) is the true "session
							// revoked" signal and terminates the stream. A transient refresh
							// failure (timeout/TypeError/5xx) is NOT terminal — it re-enters
							// this same backed-off cycle. A 401 on the reconnect itself
							// (after a successful refresh) is ambiguous (propagation lag,
							// multi-tab rotation) and also simply re-enters this cycle
							// (falls into the `unauthorized` branch again next attempt).
							//
							// Note: dedupedRefresh dedup is per-tab (per JS context) — N tabs
							// waking from expiry issue N refreshes; correct (refresh-token
							// cookie) and pre-existing; cross-tab coordination is out of scope.
							void dedupedRefresh().then(
								(refreshed) => {
									// dedupedRefresh returns the result but does NOT write the
									// token store — both existing consumers set it themselves
									// (client.ts refreshAndRetry, raw.ts). Without this line the
									// reconnect re-reads the STALE token and loops on 401.
									setAccessToken(refreshed.access_token);
									connect();
								},
								(refreshErr) => {
									if (isAuthClassRefreshFailure(refreshErr)) {
										callbacks.onStateChange?.('error');
										return;
									}
									// Transient refresh failure: not terminal. Re-enter the
									// normal reconnect cycle (attempt already incremented above;
									// the next catch's backoff/refresh handles retry).
									connect();
								}
							);
						} else {
							connect();
						}
					}, delay);
				} else {
					callbacks.onStateChange?.('error');
				}
			});
	}

	connect();

	return () => {
		disconnected = true;
		abortController?.abort();
		callbacks.onStateChange?.('disconnected');
	};
}

/**
 * Read the admin event SSE stream, dispatching each event to the callback.
 * Resolves when the stream ends (server closes).
 */
async function readAdminEventStream(body: ReadableStream<Uint8Array>, callbacks: AdminEventCallbacks): Promise<void> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';

	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;

			buffer += decoder.decode(value, { stream: true });

			let boundaryIndex: number;
			while ((boundaryIndex = buffer.indexOf('\n\n')) !== -1) {
				const eventText = buffer.slice(0, boundaryIndex);
				buffer = buffer.slice(boundaryIndex + 2);

				const event = parseSseEvent(eventText);
				if (!event) continue;

				try {
					const data: Record<string, unknown> = JSON.parse(event.data);
					callbacks.onEvent?.(event.type as AdminEventType, data);
				} catch {
					// Skip malformed events.
				}
			}
		}
	} finally {
		reader.releaseLock();
	}
}

/** Parse a single SSE event block into type + data. */
function parseSseEvent(text: string): ParsedSseEvent | null {
	let type = 'message';
	const dataLines: string[] = [];

	for (const line of text.split('\n')) {
		if (line.startsWith(':')) continue; // Comment.

		if (line.startsWith('event:')) {
			type = line.slice(6).trim();
		} else if (line.startsWith('data:')) {
			const value = line.slice(5);
			dataLines.push(value.startsWith(' ') ? value.slice(1) : value);
		}
	}

	if (dataLines.length === 0) return null;

	return { type, data: dataLines.join('\n') };
}

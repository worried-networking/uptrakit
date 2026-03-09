/**
 * SSE (Server-Sent Events) connection utility.
 *
 * Uses `fetch()` with streaming `ReadableStream` instead of `EventSource` to
 * support custom headers (Authorization: Bearer).
 */

import { getAccessToken } from './auth.svelte';

const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';

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

				const message = err instanceof Error ? err.message : 'Connection failed';
				callbacks.onError?.(message);

				// Attempt reconnection with exponential backoff.
				attempt++;
				if (attempt <= maxAttempts) {
					const delay = Math.min(1000 * Math.pow(2, attempt - 1), 30000);
					callbacks.onStateChange?.('connecting');
					setTimeout(connect, delay);
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
export type AdminEventType =
	| 'host_updated'
	| 'host_created'
	| 'host_deleted'
	| 'service_status_changed'
	| 'software_item_updated'
	| 'software_item_created'
	| 'version_check_completed'
	| 'update_started'
	| 'update_completed'
	| 'discovery_completed'
	| 'batch_update_completed'
	| 'system_service_status_changed'
	| 'scheduler_task_completed'
	| 'host_tag_created'
	| 'host_tag_updated'
	| 'host_tag_deleted'
	| 'host_tags_changed';

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

				const message = err instanceof Error ? err.message : 'Connection failed';
				callbacks.onError?.(message);

				attempt++;
				if (attempt <= maxAttempts) {
					const delay = Math.min(1000 * Math.pow(2, attempt - 1), 30000);
					callbacks.onStateChange?.('connecting');
					setTimeout(connect, delay);
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

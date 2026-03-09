/**
 * Centralised admin event store backed by a single SSE connection.
 *
 * Pages subscribe to specific event types via `subscribeToEvent()`. The SSE
 * connection is lazily created on the first subscriber and torn down when the
 * last subscriber leaves.
 *
 * Rapid duplicate events (same type + entity ID within 200ms) are debounced
 * so that a burst of, e.g., `software_item_updated` for the same item only
 * triggers one callback.
 */

import { SvelteMap } from 'svelte/reactivity';
import { connectEventStream, type AdminEventType, type SseConnectionState } from '$lib/sse';

type EventCallback = (data: Record<string, unknown>) => void;

interface Subscription {
	eventType: AdminEventType;
	callback: EventCallback;
}

/** Debounce window in milliseconds. */
const DEBOUNCE_MS = 200;

let subscriptions: Subscription[] = [];
let disconnect: (() => void) | null = null;
let connectionState: SseConnectionState = $state('disconnected');

/** Recent event fingerprints for deduplication: "type:entityId" → timer */
const debounceTimers = new SvelteMap<string, ReturnType<typeof setTimeout>>();

export function getConnectionState(): SseConnectionState {
	return connectionState;
}

/**
 * Subscribe to a specific admin event type.
 *
 * Returns an unsubscribe function. When the last subscriber leaves, the SSE
 * connection is automatically closed.
 */
export function subscribeToEvent(eventType: AdminEventType, callback: EventCallback): () => void {
	const sub: Subscription = { eventType, callback };
	subscriptions = [...subscriptions, sub];

	// Lazily open the SSE connection.
	if (subscriptions.length === 1) {
		openConnection();
	}

	return () => {
		subscriptions = subscriptions.filter((s) => s !== sub);
		if (subscriptions.length === 0) {
			closeConnection();
		}
	};
}

function openConnection() {
	disconnect = connectEventStream(
		{
			onEvent(eventType, data) {
				dispatchEvent(eventType, data);
			},
			onStateChange(state) {
				connectionState = state;
			},
			onError() {
				// Errors are handled internally by reconnection logic.
			}
		},
		{ maxReconnectAttempts: Infinity }
	);
}

function closeConnection() {
	disconnect?.();
	disconnect = null;
	connectionState = 'disconnected';
	// Clear all debounce timers.
	for (const timer of debounceTimers.values()) {
		clearTimeout(timer);
	}
	debounceTimers.clear();
}

/**
 * Dispatch an SSE event to matching subscribers with debouncing.
 *
 * Entity ID is extracted from `data.id`, `data.host_id`, or `data.task_id`
 * (whichever is present). Events with the same type + entity ID within
 * {@link DEBOUNCE_MS} are collapsed into a single callback invocation.
 */
function dispatchEvent(eventType: AdminEventType, data: Record<string, unknown>) {
	const entityId = (data.id ?? data.host_id ?? data.task_id ?? '') as string;
	const key = `${eventType}:${entityId}`;

	// Clear existing debounce for this key.
	const existing = debounceTimers.get(key);
	if (existing) {
		clearTimeout(existing);
	}

	const timer = setTimeout(() => {
		debounceTimers.delete(key);
		for (const sub of subscriptions) {
			if (sub.eventType === eventType) {
				sub.callback(data);
			}
		}
	}, DEBOUNCE_MS);

	debounceTimers.set(key, timer);
}

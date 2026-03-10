/**
 * Interactive WebSocket session client.
 *
 * Connects to the bidirectional `/api/v1/update-history/{id}/interactive`
 * endpoint. Text frames carry JSON messages (output, completed, errors).
 * Binary frames sent by the client are forwarded as raw stdin to the PTY.
 */

import { getAccessToken } from './auth.svelte';
import type { OutputLineEvent, CompletedEvent } from './sse';

export type { OutputLineEvent, CompletedEvent };

const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';

/** Connection states for the interactive session. */
export type InteractiveConnectionState = 'connecting' | 'connected' | 'completed' | 'error' | 'disconnected';

/** Callbacks for the interactive WebSocket session. */
export interface InteractiveCallbacks {
	onOutput?: (line: OutputLineEvent) => void;
	onCompleted?: (event: CompletedEvent) => void;
	/** Called when the process appears to be waiting for user input. */
	onStdinAttention?: (hint: string | null) => void;
	onStateChange?: (state: InteractiveConnectionState) => void;
	onError?: (message: string) => void;
}

/** Handle returned by {@link connectInteractiveSession}. */
export interface InteractiveHandle {
	/** Send raw input to the process stdin (e.g. xterm `onData` payload). */
	sendInput: (data: string) => void;
	/** Send a POSIX signal to the process (e.g. 2 = SIGINT). */
	sendSignal: (signal: number) => void;
	/** Cleanly close the WebSocket connection. */
	disconnect: () => void;
}

function buildWsUrl(updateHistoryId: string, token: string): string {
	const path = `${BASE}/update-history/${updateHistoryId}/interactive?token=${encodeURIComponent(token)}`;
	if (path.startsWith('http')) {
		return path.replace(/^http/, 'ws');
	}
	const wsProtocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
	return `${wsProtocol}//${location.host}${path}`;
}

/**
 * Open a bidirectional WebSocket session for an in-progress update.
 *
 * No auto-reconnect — interactive sessions are stateful and reconnection
 * would orphan the underlying PTY process.
 */
export function connectInteractiveSession(updateHistoryId: string, callbacks: InteractiveCallbacks): InteractiveHandle {
	const token = getAccessToken() ?? '';
	const url = buildWsUrl(updateHistoryId, token);

	callbacks.onStateChange?.('connecting');

	const ws = new WebSocket(url);

	ws.onopen = () => {
		callbacks.onStateChange?.('connected');
	};

	ws.onmessage = (event: MessageEvent) => {
		if (typeof event.data !== 'string') return;

		let msg: Record<string, unknown>;
		try {
			msg = JSON.parse(event.data) as Record<string, unknown>;
		} catch {
			return; // Ignore malformed frames.
		}

		const type = msg.type;

		if (type === 'output') {
			const line = msg as unknown as OutputLineEvent;
			callbacks.onOutput?.(line);
		} else if (type === 'completed') {
			const completed = msg as unknown as CompletedEvent;
			callbacks.onCompleted?.(completed);
			callbacks.onStateChange?.('completed');
		} else if (type === 'stdin_attention') {
			const hint = typeof msg.hint === 'string' ? msg.hint : null;
			callbacks.onStdinAttention?.(hint);
		} else if (type === 'error') {
			const message = typeof msg.message === 'string' ? msg.message : 'Unknown error';
			callbacks.onError?.(message);
		}
	};

	ws.onerror = () => {
		callbacks.onStateChange?.('error');
		callbacks.onError?.('WebSocket connection error');
	};

	ws.onclose = (event: CloseEvent) => {
		if (event.code === 1000 || event.code === 1001) {
			// Normal close — do not overwrite 'completed' state if already set.
			callbacks.onStateChange?.('disconnected');
		} else {
			callbacks.onStateChange?.('disconnected');
		}
	};

	const encoder = new TextEncoder();

	return {
		sendInput(data: string) {
			if (ws.readyState !== WebSocket.OPEN) return;
			// Send raw stdin as a binary frame — the server treats it as PTY input.
			ws.send(encoder.encode(data));
		},

		sendSignal(signal: number) {
			if (ws.readyState !== WebSocket.OPEN) return;
			ws.send(JSON.stringify({ type: 'signal', signal }));
		},

		disconnect() {
			ws.onclose = null; // Suppress state callback on intentional close.
			ws.close(1000, 'Client disconnected');
			callbacks.onStateChange?.('disconnected');
		}
	};
}

import '@testing-library/jest-dom';
import { afterEach } from 'vitest';

// Suppress expected console.error output from components that log hydration
// failures as part of their error-handling path. Tests that deliberately
// trigger these failures (e.g. to verify error UI) would otherwise produce
// noisy stderr that obscures real test failures.
const originalConsoleError = console.error;
console.error = (...args: unknown[]) => {
	const message = typeof args[0] === 'string' ? args[0] : '';
	if (message.startsWith('Failed to hydrate data source')) {
		return;
	}
	originalConsoleError(...args);
};

// jsdom does not implement HTMLCanvasElement.getContext(). Stub it out so
// components that render a canvas (e.g. chart libraries) don't produce
// "Not implemented" warnings in every test run.
HTMLCanvasElement.prototype.getContext = () => null;

// Overlay primitives portal their root nodes to <body> via use:portal so they
// escape the app shell's containing block. Testing Library's cleanup() unmounts
// components and fires the action's destroy cleanup, which removes the portaled
// node. This guard catches regressions where a test path skips cleanup or a
// future action implementation forgets to detach.
const PORTALED_OVERLAY_SELECTOR = [
	'[data-ui="modal-backdrop"]',
	'[data-ui="modal-shell"]',
	'[data-ui="terminal-backdrop"]',
	'[data-ui="context-menu-shell"]',
	'[data-ui="toast-notifications"]',
	'[data-ui="batch-action-bar"]'
].join(', ');

afterEach(() => {
	const leaked = document.body.querySelectorAll(PORTALED_OVERLAY_SELECTOR);
	if (leaked.length > 0) {
		const types = Array.from(leaked, (node) => node.getAttribute('data-ui')).join(', ');
		// Clean up to avoid cascading failures in subsequent tests.
		leaked.forEach((node) => node.remove());
		throw new Error(
			`Portaled overlay nodes leaked from a previous test: ${types}. Ensure cleanup() runs (Testing Library's afterEach) and the use:portal action's destroy is invoked.`
		);
	}
});

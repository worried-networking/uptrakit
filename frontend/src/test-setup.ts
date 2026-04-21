import '@testing-library/jest-dom';

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

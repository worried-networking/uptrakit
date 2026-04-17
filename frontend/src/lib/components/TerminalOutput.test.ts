import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import TerminalOutput from './TerminalOutput.svelte';

const xtermMocks = vi.hoisted(() => {
	class MockTerminal {
		options: Record<string, unknown>;
		loadAddon = vi.fn();
		open = vi.fn();
		onData = vi.fn((handler: (data: string) => void) => {
			this.onDataHandler = handler;
			return { dispose: vi.fn() };
		});
		write = vi.fn();
		clear = vi.fn();
		dispose = vi.fn();
		onDataHandler: ((data: string) => void) | null = null;

		constructor(options: Record<string, unknown>) {
			this.options = options;
			xtermMocks.terminalInstances.push(this);
		}
	}

	class MockFitAddon {
		fit = vi.fn();
	}

	class MockWebLinksAddon {}

	return {
		terminalInstances: [] as MockTerminal[],
		MockTerminal,
		MockFitAddon,
		MockWebLinksAddon
	};
});

vi.mock('@xterm/xterm', () => ({
	Terminal: xtermMocks.MockTerminal
}));

vi.mock('@xterm/addon-fit', () => ({
	FitAddon: xtermMocks.MockFitAddon
}));

vi.mock('@xterm/addon-web-links', () => ({
	WebLinksAddon: xtermMocks.MockWebLinksAddon
}));

beforeAll(() => {
	class ResizeObserverMock {
		observe = vi.fn();
		disconnect = vi.fn();
	}

	vi.stubGlobal('ResizeObserver', ResizeObserverMock);
});

afterAll(() => {
	vi.unstubAllGlobals();
});

afterEach(() => {
	cleanup();
	xtermMocks.terminalInstances.length = 0;
	vi.clearAllMocks();
});

describe('TerminalOutput', () => {
	it('renders a shared section-card shell and captured status badge', () => {
		const { container } = render(TerminalOutput, { output: 'completed output' });

		expect(container.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
		expect(screen.getByText('Captured')).toBeInTheDocument();
		expect(screen.queryByText('Interactive input enabled')).not.toBeInTheDocument();
	});

	it('shows interactive callout and forwards stdin in live mode', () => {
		const onInput = vi.fn();
		render(TerminalOutput, { onInput });

		expect(screen.getByText('Live')).toBeInTheDocument();
		expect(screen.getByText('Interactive input enabled')).toBeInTheDocument();

		const terminal = xtermMocks.terminalInstances[0];
		expect(terminal).toBeDefined();
		terminal.onDataHandler?.('help\n');
		expect(onInput).toHaveBeenCalledWith('help\n');
	});
});

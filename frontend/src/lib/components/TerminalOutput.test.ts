import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import TerminalOutput from './TerminalOutput.svelte';

const xtermMocks = vi.hoisted(() => {
	class MockTerminal {
		options: Record<string, unknown>;
		loadAddon = vi.fn();
		open = vi.fn();
		onData = vi.fn((handler: (data: string) => void) => {
			this.onDataHandler = handler;
			const dispose = vi.fn(() => {
				if (this.onDataHandler === handler) {
					this.onDataHandler = null;
				}
			});
			return { dispose };
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
	it('renders canonical terminal modal chrome with traffic lights and status bar metadata', async () => {
		const sendSigint = vi.fn();
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Live',
				statusTone: 'info',
				metadata: 'host-one · started just now · 0m',
				output: 'completed output',
				actions: [{ label: 'Ctrl+C', tone: 'danger', onclick: sendSigint }],
				onclose: vi.fn()
			} as never
		);

		expect(document.querySelector('[data-ui="terminal-backdrop"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-titlebar"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-statusbar"]')).toBeInTheDocument();
		expect(screen.getByText('Demo App on host-one')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Close terminal' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Minimize terminal' })).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Maximize terminal' })).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Ctrl+C' }));
		expect(sendSigint).toHaveBeenCalledTimes(1);
	});

	it('supports close paths and maximize toggle contract', async () => {
		const onclose = vi.fn();
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Live',
				statusTone: 'info',
				metadata: 'host-one · started just now · 0m',
				onclose
			} as never
		);

		const shell = document.querySelector('[data-ui="terminal-shell"]');
		expect(shell).toHaveAttribute('data-maximized', 'false');

		await fireEvent.click(screen.getByRole('button', { name: 'Maximize terminal' }));
		expect(shell).toHaveAttribute('data-maximized', 'true');
		expect(screen.getByRole('button', { name: 'Restore terminal' })).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Close terminal' }));
		expect(onclose).toHaveBeenCalledTimes(1);

		const backdrop = document.querySelector('[data-ui="terminal-backdrop"]');
		expect(backdrop).toBeInTheDocument();
		await fireEvent.click(backdrop as HTMLElement);
		expect(onclose).toHaveBeenCalledTimes(2);

		await fireEvent.keyDown(window, { key: 'Escape' });
		expect(onclose).toHaveBeenCalledTimes(3);
	});

	it('switches from live to captured mode without retaining stale stdin subscriptions', async () => {
		const onInput = vi.fn();
		const rendered = render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Live',
				statusTone: 'info',
				metadata: 'host-one · started just now · 0m',
				onInput,
				onclose: vi.fn()
			} as never
		);

		const terminal = xtermMocks.terminalInstances[0];
		expect(terminal).toBeDefined();
		expect(terminal.options.disableStdin).toBe(false);
		terminal.onDataHandler?.('help\n');
		expect(onInput).toHaveBeenCalledWith('help\n');
		const firstSubscription = terminal.onData.mock.results[0]?.value as { dispose: ReturnType<typeof vi.fn> };
		expect(firstSubscription.dispose).toBeDefined();

		await rendered.rerender({
			open: true,
			title: 'Demo App on host-one',
			statusLabel: 'Completed',
			statusTone: 'success',
			metadata: 'host-one · started 1m ago · 1m',
			output: 'captured output',
			onInput: null,
			onclose: vi.fn()
		} as never);

		await waitFor(() => {
			expect(firstSubscription.dispose).toHaveBeenCalledTimes(1);
			expect(terminal.onData).toHaveBeenCalledTimes(1);
			expect(terminal.onDataHandler).toBeNull();
		});
		terminal.onDataHandler?.('should-not-forward');
		expect(onInput).toHaveBeenCalledTimes(1);
	});

	it('passes the same TERMINAL_THEME reference from the module to xterm', async () => {
		const { TERMINAL_THEME } = await import('../../theme/terminal-palette');
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Captured',
				statusTone: 'neutral',
				metadata: 'host-one · started just now · 0m',
				output: 'completed output',
				onclose: vi.fn()
			} as never
		);

		const terminal = xtermMocks.terminalInstances.at(-1);
		expect(terminal?.options.theme).toBe(TERMINAL_THEME);
	});

	it('renders a single critical banner without using Callout markup', async () => {
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Queued',
				statusTone: 'warning',
				metadata: 'host-one · started just now · 0m',
				criticalBanner: {
					tone: 'warning',
					label: 'Output truncated',
					message: 'Only the first 50 MB is stored.'
				},
				onclose: vi.fn()
			} as never
		);

		expect(screen.getByText('Output truncated')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-critical-banner"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-shell"] [data-ui="callout"]')).not.toBeInTheDocument();
	});

	it('keeps details collapsed until explicitly opened', async () => {
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Completed',
				statusTone: 'success',
				metadata: 'host-one · started just now · 0m',
				details: [
					{ id: 'actor', label: 'Actor', value: 'user (actor-1)' },
					{ id: 'recovery', label: 'Recovery hint', value: 'Retry after fixing permissions.' }
				],
				onclose: vi.fn()
			} as never
		);

		expect(screen.queryByText('user (actor-1)')).not.toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: /details/i }));
		expect(screen.getByText('user (actor-1)')).toBeInTheDocument();
		expect(screen.getByText('Retry after fixing permissions.')).toBeInTheDocument();
	});

	it('renders an empty state without mounting xterm when there is no live session and no output', async () => {
		render(
			TerminalOutput as never,
			{
				open: true,
				title: 'Demo App on host-one',
				statusLabel: 'Queued',
				statusTone: 'warning',
				metadata: 'host-one · started just now · 0m',
				showTerminal: false,
				emptyState: {
					label: 'Queued',
					message: 'Waiting for another update on this host to finish.'
				},
				onclose: vi.fn()
			} as never
		);

		expect(screen.getByText('Waiting for another update on this host to finish.')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-empty-state"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="terminal-output"]')).not.toBeInTheDocument();
	});
});

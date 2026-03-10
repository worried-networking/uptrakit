<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import '@xterm/xterm/css/xterm.css';

	interface Props {
		/** Static output to render (used for completed updates). */
		output?: string;
		/** Additional CSS classes for the container. */
		class?: string;
		/**
		 * When provided, stdin is enabled and each keypress / paste in the
		 * terminal calls this handler with the raw xterm data string.
		 */
		onInput?: (data: string) => void;
	}

	let { output, class: className = '', onInput }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state(undefined);
	let terminal: Terminal | null = null;
	let fitAddon: FitAddon | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let themeObserver: MutationObserver | null = null;

	const DARK_THEME = {
		background: '#1e1e2e',
		foreground: '#cdd6f4',
		cursor: '#f5e0dc',
		selectionBackground: '#585b70',
		black: '#45475a',
		red: '#f38ba8',
		green: '#a6e3a1',
		yellow: '#f9e2af',
		blue: '#89b4fa',
		magenta: '#f5c2e7',
		cyan: '#94e2d5',
		white: '#bac2de',
		brightBlack: '#585b70',
		brightRed: '#f38ba8',
		brightGreen: '#a6e3a1',
		brightYellow: '#f9e2af',
		brightBlue: '#89b4fa',
		brightMagenta: '#f5c2e7',
		brightCyan: '#94e2d5',
		brightWhite: '#a6adc8'
	};

	const LIGHT_THEME = {
		background: '#eff1f5',
		foreground: '#4c4f69',
		cursor: '#dc8a78',
		selectionBackground: '#acb0be',
		black: '#5c5f77',
		red: '#d20f39',
		green: '#40a02b',
		yellow: '#df8e1d',
		blue: '#1e66f5',
		magenta: '#ea76cb',
		cyan: '#179299',
		white: '#acb0be',
		brightBlack: '#6c6f85',
		brightRed: '#d20f39',
		brightGreen: '#40a02b',
		brightYellow: '#df8e1d',
		brightBlue: '#1e66f5',
		brightMagenta: '#ea76cb',
		brightCyan: '#179299',
		brightWhite: '#bcc0cc'
	};

	function isDarkMode(): boolean {
		return document.documentElement.classList.contains('dark');
	}

	function getTheme() {
		return isDarkMode() ? DARK_THEME : LIGHT_THEME;
	}

	onMount(() => {
		if (!containerEl) return;

		terminal = new Terminal({
			disableStdin: onInput === undefined,
			convertEol: true,
			scrollback: 10000,
			fontSize: 13,
			fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
			theme: getTheme(),
			cursorBlink: onInput !== undefined,
			cursorStyle: 'bar',
			cursorInactiveStyle: 'none'
		});

		fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);
		terminal.loadAddon(new WebLinksAddon());

		terminal.open(containerEl);
		fitAddon.fit();

		if (onInput) {
			terminal.onData(onInput);
		}

		// Auto-resize when the container changes size.
		resizeObserver = new ResizeObserver(() => {
			fitAddon?.fit();
		});
		resizeObserver.observe(containerEl);

		// Sync theme with dark/light mode changes.
		themeObserver = new MutationObserver(() => {
			if (terminal) {
				terminal.options.theme = getTheme();
			}
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class']
		});

		// Write static output if provided.
		if (output) {
			terminal.write(output);
		}
	});

	onDestroy(() => {
		resizeObserver?.disconnect();
		themeObserver?.disconnect();
		terminal?.dispose();
	});

	// When `output` prop changes, rewrite the terminal.
	$effect(() => {
		if (terminal && output !== undefined) {
			terminal.clear();
			terminal.write(output);
		}
	});

	/** Write data to the terminal (for streaming mode). */
	export function write(data: string) {
		terminal?.write(data);
	}

	/** Clear the terminal. */
	export function clear() {
		terminal?.clear();
	}
</script>

<div bind:this={containerEl} class="terminal-output {className}"></div>

<style>
	.terminal-output {
		min-height: 200px;
		border-radius: 0.5rem;
		overflow: hidden;
	}

	.terminal-output :global(.xterm) {
		padding: 0.5rem;
	}
</style>
